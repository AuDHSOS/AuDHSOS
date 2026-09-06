// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

//! The root task: the first program, and the one that starts every other.
//!
//! It is given everything — system control, the boot image, and one memory
//! object per free region of memory — and it gives each program it starts
//! the least it needs. What it keeps for itself is what it needs to keep
//! starting things: the archive, one region to bring the memory server up
//! out of, and the endpoint every child reports its faults to.
//!
//! The order matters. The memory server goes first, out of a region this
//! program keeps back for exactly that; everything else is asked of the
//! memory server, so the boot of the system is the first three tests of it.
//! Then the name server, then the console driver — which is where the
//! kernel gives the serial port up — and last the application.
//!
//! Whoever hands memory to another process zeroes it first (D-12). Before
//! the memory server runs, that is this program's own business.

#![no_std]
#![no_main]
#![allow(unsafe_code)]
#![deny(unsafe_op_in_unsafe_fn)]

// The package holds five programs and each uses a different part of
// what it depends on; these are the crates this one does not.
use driver_uart16550 as _;
use server_console as _;
use server_memory as _;
use server_name as _;
use user_proto as _;

use audhsos_abi::layout::{MAX_MESSAGE_HANDLES, PAGE_SIZE};
use audhsos_abi::startup::{Role, Writer};
use audhsos_abi::{Error, Handle, Rights};
use user_loader::tar::{Archive, Kind};
use user_loader::{Plan, STACK_PAGES};
use user_programs::client::allocate;
use user_programs::mapping::{Mapping, SCRATCH};
use user_programs::{permissions, priority};
use user_rt::{
    EndpointHandle, InterruptHandle, IoPortHandle, Line, MemoryHandle, ProcessHandle, Startup,
    SystemControlHandle, Typed,
};
use user_sys_x86_64::{self as sys, Gate};

sys::program!(main);

/// What a program of the archive is given besides its own endpoint.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Grant {
    /// Nothing beyond the endpoints every program gets.
    None,
    /// Every memory object this program kept back, which is what makes the
    /// memory server the memory server.
    Ram,
    /// The serial port and the line it interrupts on.
    Serial,
}

/// One line of the start table: what to start, how, and with what.
#[derive(Clone, Copy, Debug)]
struct Program {
    /// The name of the file in the archive.
    name: &'static [u8],
    /// The priority its threads run at.
    priority: u8,
    /// How many handles its table holds.
    handles: u64,
    /// How many frames it may take out of the kernel reserve, which is what
    /// its page tables and its kernel stacks cost.
    frames: u64,
    /// How many kernel objects it may make.
    objects: u64,
    /// What it is given.
    grant: Grant,
    /// Whether it may ask the name server.
    names: bool,
    /// Whether it may ask the memory server.
    memory: bool,
}

/// The programs of this system, in the order they are started.
///
/// The quotas are what the programs measured out at need with room over
/// them; a program that asks for more than its line says is refused by the
/// kernel and not by this table.
const PROGRAMS: [Program; 4] = [
    Program {
        name: b"server-memory",
        priority: priority::SERVER,
        handles: 512,
        frames: 512,
        objects: 1024,
        grant: Grant::Ram,
        names: false,
        memory: false,
    },
    Program {
        name: b"server-name",
        priority: priority::SERVER,
        handles: 64,
        frames: 64,
        objects: 64,
        grant: Grant::None,
        names: false,
        memory: true,
    },
    Program {
        name: b"server-console",
        priority: priority::DRIVER,
        handles: 64,
        frames: 64,
        objects: 64,
        grant: Grant::Serial,
        names: true,
        memory: true,
    },
    Program {
        name: b"app-hello",
        priority: priority::APPLICATION,
        handles: 32,
        frames: 32,
        objects: 32,
        grant: Grant::None,
        names: true,
        memory: true,
    },
];

/// The first port of the serial controller and how many there are.
const COM1: u64 = 0x3F8;
const COM1_PORTS: u64 = 8;

/// The interrupt line the controller raises on.
const COM1_LINE: u64 = 4;

/// Where the boot image is mapped while the archive is read out of it.
const IMAGE: u64 = 0x0000_2000_0000_0000;

/// Starts the system and then answers faults for as long as it runs.
#[expect(
    clippy::needless_pass_by_value,
    reason = "the shape of `main` is what `program!` calls; the gate and the startup message belong to the program"
)]
fn main(mut gate: Gate, startup: Startup) -> ! {
    let Some(own) = startup.own_process else {
        gate.thread_exit()
    };
    let Ok(faults) = gate.endpoint_create() else {
        gate.thread_exit()
    };
    let mut world = World {
        own,
        faults,
        system: startup.system_control,
        names: None,
        memory: None,
        console: None,
        log: None,
        next_badge: 1,
    };

    let image = map_image(&mut gate, &startup, own);
    match image {
        Some(mut image) => {
            start_everything(&mut gate, &startup, &mut world, &mut image);
            let _unmapped = image.unmap(&mut gate, own);
        }
        None => say(
            &mut gate,
            &world,
            b"[init] the boot image is not readable\n",
        ),
    }
    serve_faults(&mut gate, &world)
}

/// What the root task knows about the system it has started.
struct World {
    own: ProcessHandle,
    faults: EndpointHandle,
    system: Option<SystemControlHandle>,
    names: Option<EndpointHandle>,
    memory: Option<EndpointHandle>,
    console: Option<EndpointHandle>,
    log: Option<EndpointHandle>,
    next_badge: u64,
}

impl World {
    /// The badge the next child reports its faults under.
    const fn badge(&mut self) -> u64 {
        let badge = self.next_badge;
        self.next_badge = self.next_badge.wrapping_add(1);
        badge
    }
}

/// Maps the boot image so that the archive in it can be read.
fn map_image(gate: &mut Gate, startup: &Startup, own: ProcessHandle) -> Option<Mapping> {
    let object = startup.boot_image?;
    let info = gate.memory_info(object).ok()?;
    Mapping::new(gate, own, object, IMAGE, info.length).ok()
}

/// Starts every program of the table, in order.
fn start_everything(gate: &mut Gate, startup: &Startup, world: &mut World, image: &mut Mapping) {
    let len = image.len();
    // SAFETY: the image is mapped and nothing else of this program holds a
    // reference to those bytes.
    let bytes = unsafe { image.bytes() };
    let Ok(header) = audhsos_abi::BootImageHeader::parse(bytes, len, len) else {
        say(gate, world, b"[init] the boot image header is not one\n");
        return;
    };
    let from = usize::try_from(header.archive_offset).unwrap_or(usize::MAX);
    let to = from.saturating_add(usize::try_from(header.archive_len).unwrap_or(0));
    let Some(archive) = bytes.get(from..to) else {
        say(gate, world, b"[init] the archive is not inside the image\n");
        return;
    };
    let archive = Archive::new(archive);

    // One region kept back, so that the memory server can be brought up out
    // of it; every other region goes to the memory server.
    let mut reserve = startup.ram.iter().copied().next();
    for program in PROGRAMS {
        let found = archive.find(program.name);
        let Ok(Some(entry)) = found else {
            say(
                gate,
                world,
                b"[init] a program of the table is not in the archive\n",
            );
            continue;
        };
        if entry.kind != Kind::File {
            continue;
        }
        let outcome = start(gate, startup, world, &program, entry.data, &mut reserve);
        report(gate, world, program.name, outcome);
    }
}

/// Starts one program.
fn start(
    gate: &mut Gate,
    startup: &Startup,
    world: &mut World,
    program: &Program,
    elf: &[u8],
    reserve: &mut Option<MemoryHandle>,
) -> Result<(), Error> {
    let plan = user_loader::plan(elf).map_err(|_| Error::InvalidArgument)?;
    let child = gate.process_create(world.own, program.handles, program.frames, program.objects)?;
    let endpoint = gate.endpoint_create()?;

    for region in plan.regions() {
        let object = take(gate, world, reserve, region.len, PAGE_SIZE)?;
        copy_region(gate, world.own, object, &region, elf)?;
        let bits = if region.write {
            permissions::WRITE
        } else if region.execute {
            permissions::EXECUTE
        } else {
            permissions::READ
        };
        map_into(gate, child, object, region.vaddr, region.len, bits)?;
    }

    let stack_bytes = STACK_PAGES.wrapping_mul(PAGE_SIZE);
    let stack = take(gate, world, reserve, stack_bytes, PAGE_SIZE)?;
    zero(gate, world.own, stack, stack_bytes)?;
    map_into(
        gate,
        child,
        stack,
        plan.stack_base,
        stack_bytes,
        permissions::WRITE,
    )?;

    let buffer = take(gate, world, reserve, PAGE_SIZE, PAGE_SIZE)?;
    let badge = world.badge();
    install_all(gate, startup, world, program, child, endpoint, buffer)?;

    let handler = badged(gate, world.faults, badge)?;
    gate.process_set_fault_handler(child, Some(handler))?;
    let thread = gate.thread_create(
        child,
        plan.entry,
        plan.stack_top,
        u64::from(program.priority),
        u64::from(program.priority),
        Some(buffer),
    )?;
    gate.thread_start(thread)?;
    remember(world, program, endpoint);
    Ok(())
}

/// Installs everything the child gets and writes the startup message into
/// the page that will be its IPC buffer.
fn install_all(
    gate: &mut Gate,
    startup: &Startup,
    world: &World,
    program: &Program,
    child: ProcessHandle,
    endpoint: EndpointHandle,
    buffer: MemoryHandle,
) -> Result<(), Error> {
    let mut mapping = Mapping::new(gate, world.own, buffer, SCRATCH, PAGE_SIZE)?;
    // SAFETY: the page is mapped and nothing else holds a reference to it.
    unsafe {
        mapping.zero();
    }
    let mut given: [(Role, Handle); 8] = [(Role::OwnProcess, Handle::MAX); 8];
    let mut count = 0usize;

    let own = gate.process_install_handle(child, child.handle(), ObjectRights::PROCESS)?;
    push(&mut given, &mut count, Role::OwnProcess, own);
    let served = gate.process_install_handle(child, endpoint.handle(), ObjectRights::RECV)?;
    push(&mut given, &mut count, Role::OwnEndpoint, served);

    if program.names
        && let Some(names) = world.names
    {
        let handle = gate.process_install_handle(child, names.handle(), ObjectRights::SEND)?;
        push(&mut given, &mut count, Role::NameServer, handle);
    }
    if program.memory
        && let Some(memory) = world.memory
    {
        let handle = gate.process_install_handle(child, memory.handle(), ObjectRights::SEND)?;
        push(&mut given, &mut count, Role::MemoryServer, handle);
    }
    if let Some(log) = world.log {
        let handle = gate.process_install_handle(child, log.handle(), ObjectRights::SEND)?;
        push(&mut given, &mut count, Role::Log, handle);
    }
    match program.grant {
        Grant::None => {}
        Grant::Ram => {
            for region in startup.ram.iter().skip(1).take(MAX_RAM) {
                let handle =
                    gate.process_install_handle(child, region.handle(), ObjectRights::MEMORY)?;
                push(&mut given, &mut count, Role::Ram, handle);
            }
        }
        Grant::Serial => {
            let (ports, line) = serial(gate, world)?;
            let handle = gate.process_install_handle(child, ports.handle(), ObjectRights::PORTS)?;
            push(&mut given, &mut count, Role::IoPorts, handle);
            let handle = gate.process_install_handle(child, line.handle(), ObjectRights::MANAGE)?;
            push(&mut given, &mut count, Role::Interrupt, handle);
        }
    }

    let mut writer = Writer::new();
    {
        // SAFETY: as above.
        let bytes = unsafe { mapping.bytes() };
        let Some(page) = bytes.first_chunk_mut::<{ audhsos_abi::ipc_buffer::SIZE }>() else {
            return Err(Error::BufferTooSmall);
        };
        let mut buffer = audhsos_abi::BufferMut::new(page);
        for (role, handle) in given.iter().take(count) {
            writer.give(&mut buffer, *role, *handle)?;
        }
        writer.finish(&mut buffer)?;
    }
    mapping.unmap(gate, world.own)
}

/// How many memory objects fit into the startup message beside the rest.
const MAX_RAM: usize = 240;

/// Records one pair of the startup message.
fn push(given: &mut [(Role, Handle); 8], count: &mut usize, role: Role, handle: Handle) {
    if let Some(slot) = given.get_mut(*count) {
        *slot = (role, handle);
        *count = count.wrapping_add(1);
    }
}

/// The rights each kind of handle goes out with.
struct ObjectRights;

impl ObjectRights {
    const PROCESS: Rights = Rights::MAP.union(Rights::MANAGE);
    const RECV: Rights = Rights::RECV.union(Rights::BADGE).union(Rights::SEND);
    const SEND: Rights = Rights::SEND.union(Rights::TRANSFER);
    const MEMORY: Rights = Rights::READ
        .union(Rights::WRITE)
        .union(Rights::MAP)
        .union(Rights::INFO)
        .union(Rights::TRANSFER);
    const PORTS: Rights = Rights::READ.union(Rights::WRITE);
    const MANAGE: Rights = Rights::MANAGE;
}

/// The port range and the interrupt of the serial controller, made once.
fn serial(gate: &mut Gate, world: &World) -> Result<(IoPortHandle, InterruptHandle), Error> {
    let system = world.system.ok_or(Error::AccessDenied)?;
    let ports = gate.ioport_create(system, COM1, COM1_PORTS)?;
    let line = gate.interrupt_create(system, COM1_LINE)?;
    Ok((ports, line))
}

/// A capability to the fault endpoint that marks what comes through it.
fn badged(gate: &mut Gate, faults: EndpointHandle, badge: u64) -> Result<EndpointHandle, Error> {
    gate.endpoint_badge(faults, badge)
}

/// Memory for a child: from the memory server once it runs, and out of the
/// region this program kept back before that.
fn take(
    gate: &mut Gate,
    world: &World,
    reserve: &mut Option<MemoryHandle>,
    len: u64,
    align: u64,
) -> Result<MemoryHandle, Error> {
    if let Some(memory) = world.memory {
        return allocate(gate, memory, len, align);
    }
    let object = reserve.ok_or(Error::OutOfKernelMemory)?;
    let bytes = len.next_multiple_of(PAGE_SIZE);
    let rest = gate.memory_split(object, bytes)?;
    *reserve = Some(rest);
    Ok(object)
}

/// Copies the bytes of one region of the program into the object that will
/// hold it, and zeroes what the file does not fill.
fn copy_region(
    gate: &mut Gate,
    own: ProcessHandle,
    object: MemoryHandle,
    region: &user_loader::Region,
    elf: &[u8],
) -> Result<(), Error> {
    let mut mapping = Mapping::new(gate, own, object, SCRATCH, region.len)?;
    let from = usize::try_from(region.file_offset).unwrap_or(usize::MAX);
    let to = from.saturating_add(usize::try_from(region.file_size).unwrap_or(0));
    let source = elf.get(from..to).unwrap_or(&[]);
    // SAFETY: the object is mapped and nothing else holds a reference to
    // those bytes.
    unsafe {
        mapping.zero();
    }
    // SAFETY: as above.
    unsafe {
        mapping.copy_in(region.leading, source);
    }
    mapping.unmap(gate, own)
}

/// Fills an object with zeros, which is what this program owes anything it
/// hands on (D-12).
fn zero(gate: &mut Gate, own: ProcessHandle, object: MemoryHandle, len: u64) -> Result<(), Error> {
    let mut mapping = Mapping::new(gate, own, object, SCRATCH, len)?;
    // SAFETY: as `copy_region`.
    unsafe {
        mapping.zero();
    }
    mapping.unmap(gate, own)
}

/// Maps `object` into `child` at `address` with `bits`.
fn map_into(
    gate: &mut Gate,
    child: ProcessHandle,
    object: MemoryHandle,
    address: u64,
    len: u64,
    bits: u64,
) -> Result<(), Error> {
    let mut done = 0u64;
    while done < len {
        let rest = len.wrapping_sub(done);
        let chunk = rest.min(audhsos_abi::layout::MAX_PAGES_PER_CALL.wrapping_mul(PAGE_SIZE));
        let pages =
            gate.memory_map(child, object, address.wrapping_add(done), done, chunk, bits)?;
        let moved = pages.wrapping_mul(PAGE_SIZE);
        if moved == 0 {
            return Err(Error::NotMapped);
        }
        done = done.wrapping_add(moved);
    }
    Ok(())
}

/// Remembers the endpoint of a server that has just started, so that the
/// programs after it can be given one.
const fn remember(world: &mut World, program: &Program, endpoint: EndpointHandle) {
    match program.name {
        b"server-memory" => world.memory = Some(endpoint),
        b"server-name" => world.names = Some(endpoint),
        b"server-console" => {
            world.console = Some(endpoint);
            world.log = Some(endpoint);
        }
        _ => {}
    }
}

/// Says what came of starting a program.
fn report(gate: &mut Gate, world: &World, name: &[u8], outcome: Result<(), Error>) {
    match outcome {
        Ok(()) => {
            let mut line: Line<96> = Line::new();
            line.put(b"[init] started ");
            line.put(name);
            line.put(b"\n");
            say(gate, world, line.as_bytes());
        }
        Err(error) => {
            let mut line: Line<160> = Line::new();
            line.put(b"[init] ");
            line.put(name);
            line.put(b": ");
            line.put(error.message().as_bytes());
            line.put(b"\n");
            say(gate, world, line.as_bytes());
        }
    }
}

/// Puts a line out: through the console once there is one, and through the
/// kernel's own before that.
fn say(gate: &mut Gate, world: &World, line: &[u8]) {
    match world.console {
        Some(console) => {
            let _said = user_programs::client::write_line(gate, console, line);
        }
        None => {
            let _logged = sys::write_line(gate, None, line);
        }
    }
}

/// Answers the faults of every child, for as long as the system runs.
///
/// A child that faulted is reported and killed. It will not get better by
/// being left standing, and what it holds is what somebody else needs.
fn serve_faults(gate: &mut Gate, world: &World) -> ! {
    loop {
        let Ok(received) = gate.ipc_recv(world.faults) else {
            gate.thread_exit()
        };
        let label = gate.reader().message().map_or(0, |message| message.label);
        let kind = audhsos_abi::ipc_buffer::fault_kind_of(label);
        let mut line: Line<128> = Line::new();
        line.put(b"[init] child ");
        put_number(&mut line, received.badge);
        line.put(b" faulted: ");
        line.put(
            kind.map_or("unknown", audhsos_abi::FaultKind::name)
                .as_bytes(),
        );
        line.put(b"\n");
        say(gate, world, line.as_bytes());
        // No reply: a thread whose fault goes unanswered stays stopped, and
        // the process goes with the handle its parent holds. There is no
        // handle here, so the thread simply stops.
        let _ = received;
    }
}

/// Writes `value` into `line` in decimal.
fn put_number(line: &mut Line<128>, value: u64) {
    let mut digits = [0u8; 20];
    let mut rest = value;
    let mut written = 0usize;
    loop {
        let digit = u8::try_from(rest.wrapping_rem(10)).unwrap_or(0);
        if let Some(slot) = digits.get_mut(written) {
            *slot = digit.wrapping_add(b'0');
        }
        written = written.wrapping_add(1);
        rest = rest.wrapping_div(10);
        if rest == 0 || written >= digits.len() {
            break;
        }
    }
    for index in (0..written).rev() {
        line.put(
            digits
                .get(index)
                .map_or(b"0".as_slice(), core::slice::from_ref),
        );
    }
}

/// The plan of a program, kept so that the type is named in this file.
type _Plan = Plan;

/// The widest message a child could be given, which the handle area bounds.
const _: () = assert!(MAX_MESSAGE_HANDLES <= 4);
