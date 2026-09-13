// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

//! The program that walks the PCI bus: it maps the configuration window the
//! root task gave it, reports every function it finds, and says what the
//! virtio network device published.
//!
//! Nothing of the bus layout is here. The `pci` crate reads the bytes
//! through a trait, `user-sys-x86_64` reaches the mapping volatile, and this
//! program is the adapter between the two plus the lines it prints. A
//! machine whose firmware published no window, or one started without the
//! network device, reports that and ends: both are runs this phase is
//! accepted on (13.12).
//!
//! One bus of a window is one mebibyte, and a window may cover all two
//! hundred and fifty-six of them. The walk therefore maps one bus at a
//! time, at the same address, and takes it back before the next: the page
//! tables of one mebibyte are what this program costs, whatever the
//! firmware published.

#![no_std]
#![no_main]
#![allow(unsafe_code)]
#![deny(unsafe_op_in_unsafe_fn)]

// The package holds thirteen programs and each uses a different part of
// what it depends on; these are the crates this one does not.
use app_canvas as _;
use audhsos_time as _;
use driver_i8042 as _;
use driver_uart16550 as _;
use driver_virtio_blk as _;
use fs_fat as _;
use gfx as _;
use server_console as _;
use server_display as _;
use server_fs as _;
use server_input as _;
use server_memory as _;
use server_name as _;
use user_loader as _;
use virtio_queue as _;

use audhsos_abi::Error;
use audhsos_abi::startup::BusRange;
use pci::address::{Address, BYTES_PER_BUS, Window};
use pci::bar::{Bar, Space, Width, probe};
use pci::capability::{Capability, ID_MSIX, ID_VENDOR, MAX_CAPABILITIES, find, walk};
use pci::enumerate::walk as enumerate;
use pci::error::PciError;
use pci::header::Kind;
use pci::msix;
use pci::virtio::{self, NETWORK_DEVICE, VIRTIO_VENDOR};
use user_programs::client::{lookup, write_line};
use user_programs::config_space::MappedSpace;
use user_programs::mapping::{Mapping, SCRATCH};
use user_proto::parent;
use user_rt::{EndpointHandle, Line, Startup};
use user_sys_x86_64::{self as sys, Gate, Mmio};

sys::program!(main);

/// The name the console driver registered itself under.
const CONSOLE: &[u8] = b"console";

/// How long a line this program writes. A function with six base address
/// registers and a capability list of its own fills most of it.
type Report = Line<256>;

/// Walks the bus and reports it.
#[expect(
    clippy::needless_pass_by_value,
    reason = "the shape of `main` is what `program!` calls; the gate and the startup message belong to the program"
)]
fn main(mut gate: Gate, startup: Startup) -> ! {
    let voice = voice(&mut gate, &startup);
    let outcome = report_bus(&mut gate, &startup, voice);
    if let Err(error) = outcome {
        say(
            &mut gate,
            voice,
            &Report::of(format_args!("[lspci] {}\n", error.message())),
        );
    }
    finish(&mut gate, &startup);
    gate.thread_exit()
}

/// The endpoint the lines go to: the console it looks up, or the log the
/// root task gave it when there is no console.
fn voice(gate: &mut Gate, startup: &Startup) -> Option<EndpointHandle> {
    let looked_up = startup
        .name_server
        .and_then(|names| lookup(gate, names, CONSOLE).ok());
    looked_up.or(startup.log)
}

/// Writes one line, if there is anywhere to write it.
///
/// A line that was cut lost its own newline to the cut, so the newline goes
/// out on its own: without it the next writer would continue on this line
/// and two reports would read as one.
fn say(gate: &mut Gate, voice: Option<EndpointHandle>, line: &Report) {
    let Some(endpoint) = voice else {
        return;
    };
    let bytes = line.as_bytes();
    let _said = write_line(gate, endpoint, bytes);
    if bytes.last() != Some(&b'\n') {
        let _ended = write_line(gate, endpoint, b"\n");
    }
}

/// Maps the window bus by bus and reports what every bus holds.
fn report_bus(
    gate: &mut Gate,
    startup: &Startup,
    voice: Option<EndpointHandle>,
) -> Result<(), Error> {
    let process = startup.own_process.ok_or(Error::AccessDenied)?;
    let (Some(memory), Some(buses)) = (startup.ecam, startup.bus_range()) else {
        say(
            gate,
            voice,
            &Report::of(format_args!("[lspci] no window\n")),
        );
        return Ok(());
    };
    say(
        gate,
        voice,
        &Report::of(format_args!(
            "[lspci] window segment={} buses={}..={}\n",
            buses.segment, buses.first_bus, buses.last_bus
        )),
    );
    let mut found = 0usize;
    let mut virtio_found = false;
    for bus in buses.first_bus..=buses.last_bus {
        let offset = u64::from(bus.saturating_sub(buses.first_bus)).saturating_mul(BYTES_PER_BUS);
        let mut mapping = Mapping::window(gate, process, memory, SCRATCH, offset, BYTES_PER_BUS)?;
        let outcome = report_one_bus(gate, voice, &mut mapping, buses, bus);
        mapping.unmap(gate, process)?;
        let (count, virtio) = outcome?;
        found = found.saturating_add(count);
        virtio_found = virtio_found || virtio;
    }
    if !virtio_found {
        say(
            gate,
            voice,
            &Report::of(format_args!("[lspci] no virtio device\n")),
        );
    }
    say(
        gate,
        voice,
        &Report::of(format_args!("[lspci] functions={found}\n")),
    );
    Ok(())
}

/// Reports the functions of one bus and says whether the virtio network
/// device was among them.
fn report_one_bus(
    gate: &mut Gate,
    voice: Option<EndpointHandle>,
    mapping: &mut Mapping,
    buses: BusRange,
    bus: u8,
) -> Result<(usize, bool), Error> {
    let window = Window::new(buses.segment, bus, bus).map_err(pci_error)?;
    // SAFETY: the mapping stands until it is unmapped by the caller, it is
    // one mebibyte of device memory of this process alone, and the value
    // built here is the only one that reaches those bytes.
    let bytes = unsafe { mapping.bytes() };
    let mut space = MappedSpace::new(window, Mmio::of(bytes));
    let mut addresses = [None; MAX_FUNCTIONS_PER_BUS];
    let mut count = 0usize;
    enumerate(&space, window, |function| {
        if let Some(slot) = addresses.get_mut(count) {
            *slot = Some(*function);
            count = count.saturating_add(1);
        }
    })
    .map_err(pci_error)?;
    let mut virtio_found = false;
    for function in addresses.iter().flatten() {
        report_function(gate, voice, &mut space, function)?;
        if function.header.vendor == VIRTIO_VENDOR && function.header.device == NETWORK_DEVICE {
            report_virtio(gate, voice, &space, function.address)?;
            virtio_found = true;
        }
    }
    Ok((count, virtio_found))
}

/// How many functions of one bus this program reports. A bus holds
/// thirty-two devices of eight functions; a walk that finds more than this
/// reports the first of them and counts no further.
const MAX_FUNCTIONS_PER_BUS: usize = 32;

/// Reports one function: where it is, what it is, and what it decodes.
fn report_function(
    gate: &mut Gate,
    voice: Option<EndpointHandle>,
    space: &mut MappedSpace<'_>,
    function: &pci::enumerate::Function,
) -> Result<(), Error> {
    let address = function.address;
    let header = &function.header;
    let mut line = Report::of(format_args!(
        "[lspci] {:02x}:{:02x}.{} {:04x}:{:04x} class={:02x}:{:02x}:{:02x}{}",
        address.bus(),
        address.device(),
        address.function(),
        header.vendor,
        header.device,
        header.class,
        header.subclass,
        header.prog_if,
        kind_name(header.kind)
    ));
    // Only a type-0 header carries six base address registers; where a
    // bridge has four of them it has its bus numbers and its windows, and
    // `pci` refuses to probe one for that reason.
    if header.kind == Kind::Endpoint {
        let bars = probe(space, address).map_err(pci_error)?;
        for bar in bars.iter().flatten() {
            put_bar(&mut line, bar);
        }
    }
    let capabilities = walk(space, address, header).map_err(pci_error)?;
    put_capabilities(&mut line, &capabilities);
    line.put(b"\n");
    say(gate, voice, &line);
    Ok(())
}

/// Reports what a virtio device published: its structures and the size of
/// its message table.
fn report_virtio(
    gate: &mut Gate,
    voice: Option<EndpointHandle>,
    space: &MappedSpace<'_>,
    address: Address,
) -> Result<(), Error> {
    let header = pci::header::read(space, address)
        .map_err(pci_error)?
        .ok_or(Error::NotFound)?;
    let capabilities = walk(space, address, &header).map_err(pci_error)?;
    let structures = virtio::structures(space, address, &capabilities).map_err(pci_error)?;
    let mut line = Report::of(format_args!("[lspci] virtio-net structures="));
    let mut first = true;
    for structure in structures.iter().flatten() {
        if !first {
            line.put(b",");
        }
        first = false;
        line.put(structure_name(structure.kind).as_bytes());
        if let Some(multiplier) = structure.multiplier {
            line.put(Report::of(format_args!("({multiplier})")).as_bytes());
        }
    }
    match find(&capabilities, ID_MSIX) {
        Some(capability) => {
            let table = msix::read(space, address, capability.offset).map_err(pci_error)?;
            line.put(Report::of(format_args!(" msix={}\n", table.vectors)).as_bytes());
        }
        None => line.put(b" msix=none\n"),
    }
    say(gate, voice, &line);
    Ok(())
}

/// Puts one base address register into the line.
fn put_bar(line: &mut Report, bar: &Bar) {
    let kind = match bar.space {
        Space::Io => "io",
        Space::Memory {
            width: Width::Bits32,
            ..
        } => "mem32",
        Space::Memory {
            width: Width::Bits64,
            ..
        } => "mem64",
    };
    line.put(
        Report::of(format_args!(
            " bar{}={kind} {:#x}+{:#x}",
            bar.index, bar.base, bar.len
        ))
        .as_bytes(),
    );
}

/// Puts the capabilities of a function into the line, by name where this
/// program knows one.
fn put_capabilities(line: &mut Report, capabilities: &[Option<Capability>; MAX_CAPABILITIES]) {
    let mut first = true;
    for capability in capabilities.iter().flatten() {
        line.put(if first { b" caps=" } else { b"," });
        first = false;
        match capability.id {
            ID_MSIX => line.put(b"msix"),
            ID_VENDOR => line.put(b"vendor"),
            other => line.put(Report::of(format_args!("{other:#04x}")).as_bytes()),
        }
    }
}

/// What a header type is called in the lines above.
const fn kind_name(kind: Kind) -> &'static str {
    match kind {
        Kind::Endpoint => "",
        Kind::Bridge => " bridge",
        Kind::Other(_) => " unread",
    }
}

/// What a virtio structure is called in the line above.
const fn structure_name(kind: virtio::Kind) -> &'static str {
    match kind {
        virtio::Kind::Common => "common",
        virtio::Kind::Notify => "notify",
        virtio::Kind::Isr => "isr",
        virtio::Kind::Device => "device",
        virtio::Kind::PciConfig => "pci",
        virtio::Kind::SharedMemory => "shared",
        virtio::Kind::Vendor => "vendor",
    }
}

/// The error a refusal of the `pci` crate becomes. The bus is what the
/// firmware left; a walk that cannot read it is a machine this program
/// cannot report on, which is `Unsupported` and no fault of a caller.
const fn pci_error(_error: PciError) -> Error {
    Error::Unsupported
}

/// Tells the process that started this one that the work is done.
fn finish(gate: &mut Gate, startup: &Startup) {
    let Some(parent) = startup.parent else {
        return;
    };
    let finished = parent::Request::Finished {
        status: parent::SUCCESS,
    };
    if finished.encode(&mut gate.writer()).is_ok() {
        let _reported = gate.ipc_send(parent);
    }
}
