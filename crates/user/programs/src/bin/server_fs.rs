// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

//! The file system server: the one program that drives the block device
//! and the one that answers the file protocol.
//!
//! It uses one thread. A disk read is synchronous — submit the chain,
//! notify the device, wait for the interrupt, drain the used ring, copy
//! the sector out — and the kernel lets a thread wait on one object at a
//! time, so the thread waits on the endpoint at the top of its loop and on
//! the notification inside the read. While one client's sector is in
//! flight another client waits; a queue of two slots produces that wait
//! anyway, and a second thread is a later version rather than a repair.
//!
//! The wait carries a deadline. Without one a device that stops answering
//! leaves the server asleep for good; with one the request is refused and
//! the queue reset.
//!
//! A machine that carries no disk starts this program all the same, and it
//! answers `Unavailable` to everything.
//!
//! Invariant: the transport is reached through one `RefCell`, and no
//! borrow of it is held across a call into `fs-fat`, which borrows it
//! again for every sector.

#![no_std]
#![no_main]
#![allow(unsafe_code)]
#![deny(unsafe_op_in_unsafe_fn)]

// The package holds fifteen programs and each uses a different part of
// what it depends on; these are the crates this one does not.
use app_canvas as _;
use audhsos_time as _;
use driver_i8042 as _;
use driver_uart16550 as _;
use gfx as _;
use pci as _;
use server_console as _;
use server_display as _;
use server_input as _;
use server_memory as _;
use server_name as _;
use user_loader as _;

use core::cell::RefCell;

use audhsos_abi::Error;
use audhsos_abi::layout::PAGE_SIZE;
use audhsos_abi::startup::Location;
use driver_virtio_blk::blk::{Blk, Vectors};
use driver_virtio_blk::request::{Kind, Request as BlkRequest, status};
use driver_virtio_blk::{BlkError, NO_VECTOR};
use fs_fat::{BlockDevice, Error as FatError, FileSystem, FormatOptions, SECTOR};
use server_fs::{Clients, answer, moment};
use user_programs::client::{allocate, register, write_line};
use user_programs::dma::{Dma, QUEUE_SIZE, REGION_BYTES, SECTOR_BYTES};
use user_programs::mapping::Mapping;
use user_programs::registers::{BLOCK_WINDOW, Window};
use user_programs::serve::{Serving, receive};
use user_proto::file::{Reply, Request};
use user_rt::{
    EndpointHandle, InterruptHandle, Line, MemoryHandle, NotificationHandle, ProcessHandle, Startup,
};
use user_sys_x86_64::{self as sys, Gate};
use virtio_queue::Queue;

sys::program!(main);

/// The name the server registers itself under.
const NAME: &[u8] = b"files";

/// Words of the free set of the queue: one bit per descriptor, which
/// [`QUEUE_SIZE`] of them fit in one.
const QUEUE_WORDS: usize = 1;

/// Where the region the device reads and writes is mapped.
const DMA: u64 = 0x0000_4C00_0000_0000;

/// The slot every request uses. One at a time is what one thread does.
const SLOT: usize = 0;

/// How long a request may take before the server gives up on the device,
/// in microseconds. A virtio device in software answers in less than a
/// millisecond; a second is far past anything but a device that stopped.
const DEADLINE: u64 = 1_000_000;

/// How long a line this program writes.
type Report = Line<160>;

/// Brings the disk up, mounts the volume, and answers clients.
///
/// The two windows are mapped here and nowhere else: the driver holds byte
/// slices over them for the rest of the program, and a borrow cannot
/// outlive the mapping it came from, so the mappings live in the frame
/// that never returns. The gate is moved into the transport for the same
/// reason of ownership — the driver waits for the interrupt through it,
/// and `Gate::adopt` allows exactly one gate per buffer.
#[expect(
    clippy::needless_pass_by_value,
    reason = "the shape of `main` is what `program!` calls; the gate and the startup message belong to the program"
)]
fn main(mut gate: Gate, startup: Startup) -> ! {
    let voice = startup.log;
    let Some(endpoint) = startup.own_endpoint else {
        gate.thread_exit()
    };
    if let Some(names) = startup.name_server {
        let _registered = register(&mut gate, names, NAME, endpoint);
    }
    let (Some(device), Some(process), Some(memory)) = (
        Given::of(&startup),
        startup.own_process,
        startup.memory_server,
    ) else {
        let line = Report::of(format_args!("[files] no disk\n"));
        if let Some(console) = voice {
            let _logged = write_line(&mut gate, console, line.as_bytes());
        }
        let _refused = refuse_everything(&mut gate, endpoint);
        gate.thread_exit()
    };

    let mut windows = match map_windows(&mut gate, process, memory, &device) {
        Ok(windows) => windows,
        Err(error) => stop(&mut gate, voice, error),
    };
    // SAFETY: both mappings stand for the rest of this program — neither
    // is unmapped and this function does not return — each is memory of
    // this process alone, and the two values built here are the only ones
    // that reach those bytes.
    let mut registers = Window::new(unsafe { windows.registers.bytes() }, device.places);
    // SAFETY: as above.
    let Some(mut dma) = Dma::new(unsafe { windows.region.bytes() }, windows.physical) else {
        stop(&mut gate, voice, Error::Unaligned)
    };

    let ready = bring_up(&mut gate, &mut dma, &mut registers, &device);
    let (blk, queue, sectors) = match ready {
        Ok(parts) => parts,
        Err(error) => stop(&mut gate, voice, error),
    };
    let mut disk = Disk {
        sectors,
        inner: RefCell::new(Transport {
            gate,
            registers,
            dma,
            blk,
            queue,
            notification: device.notification,
            interrupt: device.interrupt,
            bit: device.bit,
            voice,
        }),
    };
    let line = Report::of(format_args!("[files] disk of {sectors} sectors\n"));
    report(&disk, voice, &line);

    // The volume borrows the disk rather than taking it, because
    // `FileSystem::mount` drops the device it was given when it refuses,
    // and the gate this program says anything through is inside that
    // device.
    let outcome = match mount(&mut disk) {
        Ok(mut volume) => {
            geometry(&volume, voice);
            serve(&mut volume, endpoint, voice)
        }
        Err(error) => Err(error),
    };
    if let Err(error) = outcome {
        let line = Report::of(format_args!("[files] {}\n", error.message()));
        report(&disk, voice, &line);
    }
    let mut transport = disk.inner.borrow_mut();
    transport.gate.thread_exit()
}

/// Says why the server stops and ends the thread.
fn stop(gate: &mut Gate, voice: Option<EndpointHandle>, error: Error) -> ! {
    let line = Report::of(format_args!("[files] {}\n", error.message()));
    if let Some(console) = voice {
        let _logged = write_line(gate, console, line.as_bytes());
    }
    gate.thread_exit()
}

/// The two mappings the driver works through.
struct Windows {
    registers: Mapping,
    region: Mapping,
    physical: u64,
}

/// Maps the register window and the region the device reads and writes.
fn map_windows(
    gate: &mut Gate,
    process: ProcessHandle,
    memory: EndpointHandle,
    device: &Given,
) -> Result<Windows, Error> {
    let registers = Mapping::new(gate, process, device.registers, BLOCK_WINDOW, device.bytes)?;
    // What is asked for and what is mapped are one number, so a larger
    // queue or a second slot needs no change here.
    let bytes = u64::try_from(REGION_BYTES)
        .unwrap_or(0)
        .next_multiple_of(PAGE_SIZE);
    let object = allocate(gate, memory, bytes, PAGE_SIZE)?;
    let physical = gate.memory_info(object)?.start;
    let region = Mapping::new(gate, process, object, DMA, bytes)?;
    Ok(Windows {
        registers,
        region,
        physical,
    })
}

/// Writes one line through the gate the transport holds.
fn report(disk: &Disk<'_>, voice: Option<EndpointHandle>, line: &Report) {
    let Ok(mut transport) = disk.inner.try_borrow_mut() else {
        return;
    };
    if let Some(console) = voice {
        let _logged = write_line(&mut transport.gate, console, line.as_bytes());
    }
}

/// What the root task gave this process about the device.
struct Given {
    registers: MemoryHandle,
    bytes: u64,
    places: [Location; 4],
    multiplier: u32,
    notification: NotificationHandle,
    interrupt: InterruptHandle,
    bit: u64,
}

impl Given {
    /// What the startup message says, or nothing when the machine carries
    /// no disk.
    fn of(startup: &Startup) -> Option<Self> {
        let places = startup.block_structures()?;
        let mut end = 0u64;
        for place in &places {
            end = end.max(u64::from(place.offset).wrapping_add(u64::from(place.len)));
        }
        Some(Given {
            registers: startup.block_registers?,
            bytes: end.next_multiple_of(PAGE_SIZE),
            places,
            multiplier: u32::try_from(startup.block_notify_multiplier?).ok()?,
            notification: startup.block_notification?,
            interrupt: startup.block_interrupt?,
            bit: startup.block_vector_bit?,
        })
    }
}

/// Resets the device and brings it up: the driver, its queue, and how
/// many sectors the device has.
fn bring_up(
    gate: &mut Gate,
    dma: &mut Dma<'_>,
    registers: &mut Window<'_>,
    device: &Given,
) -> Result<(Blk, Queue<QUEUE_WORDS>, u32), Error> {
    let mut blk = Blk::new(device.multiplier);
    blk.reset(registers);
    let deadline = gate.clock_now()?.saturating_add(DEADLINE);
    while !blk.is_reset(registers) {
        if gate.clock_now()? > deadline {
            return Err(Error::Unavailable);
        }
    }
    // The three regions are zeroed before the device is told where they
    // are, which is what virtio 2.7.10.1 asks of the used ring's flags.
    dma.whole().fill(0);
    let queue = Queue::<QUEUE_WORDS>::new(dma, QUEUE_SIZE).map_err(|_| Error::InvalidState)?;
    let vectors = Vectors {
        config: NO_VECTOR,
        queue: 0,
    };
    blk.initialize(registers, &dma.rings(), &vectors)
        .map_err(device_error)?;
    let sectors = u32::try_from(blk.capacity()).map_err(|_| Error::Unsupported)?;
    Ok((blk, queue, sectors))
}

/// What a refusal of the driver becomes.
const fn device_error(_error: BlkError) -> Error {
    Error::Unavailable
}

/// The transport of one disk: the registers, the region, the queue, and
/// the gate the wait goes through.
struct Transport<'a> {
    gate: Gate,
    registers: Window<'a>,
    dma: Dma<'a>,
    blk: Blk,
    queue: Queue<QUEUE_WORDS>,
    notification: NotificationHandle,
    interrupt: InterruptHandle,
    bit: u64,
    voice: Option<EndpointHandle>,
}

/// The disk as `fs-fat` reads and writes it.
///
/// The sector count stands outside the cell: it does not change after the
/// device came up, and `sectors` has no way to report a refusal, so a
/// borrow there would be a panic where every other path answers an error.
struct Disk<'a> {
    inner: RefCell<Transport<'a>>,
    sectors: u32,
}

impl Transport<'_> {
    /// One request, from the chain to the status byte.
    fn request(&mut self, kind: Kind, sector: u32) -> Result<u32, FatError> {
        let request = match kind {
            Kind::In => BlkRequest::read(u64::from(sector)),
            Kind::Out => BlkRequest::write(u64::from(sector)),
            Kind::Flush => BlkRequest::flush(),
        };
        self.dma.clear_status(SLOT);
        request
            .write_header(self.dma.header(SLOT))
            .map_err(|_| FatError::Device(sector))?;
        let chain = self.dma.chain(SLOT, kind != Kind::Flush);
        self.blk
            .submit(&mut self.queue, &mut self.dma, &request, &chain)
            .map_err(|_| FatError::Device(sector))?;
        self.blk.notify(&mut self.registers);
        self.wait(sector)
    }

    /// Waits for the device to answer, and answers how many bytes it
    /// wrote.
    ///
    /// A deadline that passes resets the queue: the device has a chain of
    /// this driver's and will not say what it did with it, so nothing of
    /// the queue can be believed afterwards.
    fn wait(&mut self, sector: u32) -> Result<u32, FatError> {
        let deadline = self
            .gate
            .clock_now()
            .map_err(|_| FatError::Device(sector))?
            .saturating_add(DEADLINE);
        let mask = 1u64.wrapping_shl(u32::try_from(self.bit).unwrap_or(0));
        loop {
            if self.gate.clock_now().is_ok_and(|now| now >= deadline) {
                return Err(self.give_up(sector));
            }
            let bits = self
                .gate
                .notification_wait_until(self.notification, deadline)
                .map_err(|_| self.give_up(sector))?;
            let _acked = self.gate.interrupt_ack(self.interrupt);
            // A wake-up whose bit is another one is not this device's. The
            // deadline at the top of the loop is what ends a wait that
            // never gets the right bit.
            if bits & mask == 0 {
                continue;
            }
            // A device that asked for a reset says so in the status, and
            // nothing of the queue can be believed afterwards.
            if self.blk.poll(&mut self.registers).is_err() {
                return Err(self.give_up(sector));
            }
            match self.queue.next_used(self.blk.device(), &self.dma) {
                Ok(Some(completion)) => {
                    return match status(self.dma.status(SLOT)) {
                        Ok(()) => Ok(completion.length),
                        Err(_) => Err(FatError::Device(sector)),
                    };
                }
                Ok(None) => {}
                Err(_) => return Err(self.give_up(sector)),
            }
        }
    }

    /// Writes one line of the driver's own.
    fn trace(&mut self, what: core::fmt::Arguments<'_>) {
        let Some(console) = self.voice else {
            return;
        };
        let line = Report::of(what);
        let _said = write_line(&mut self.gate, console, line.as_bytes());
    }

    /// Resets the device and the queue, and names the sector that was
    /// lost.
    fn give_up(&mut self, sector: u32) -> FatError {
        self.blk.reset(&mut self.registers);
        self.trace(format_args!("[files] sector {sector} was not answered\n"));
        FatError::Device(sector)
    }
}

impl BlockDevice for Disk<'_> {
    fn sectors(&self) -> u32 {
        self.sectors
    }

    fn read(&self, sector: u32, into: &mut [u8; SECTOR]) -> Result<(), FatError> {
        let mut transport = self
            .inner
            .try_borrow_mut()
            .map_err(|_| FatError::Device(sector))?;
        if sector >= self.sectors {
            return Err(FatError::Device(sector));
        }
        let moved = transport.request(Kind::In, sector)?;
        // The device says how many bytes it wrote, and a caller's buffer
        // is whatever the caller left in it. A device that wrote less than
        // the sector is therefore a refusal and not a short copy: the
        // bytes it did not write would otherwise be read as the disk's.
        if usize::try_from(moved).unwrap_or(0) < SECTOR_BYTES {
            return Err(FatError::Device(sector));
        }
        let slot = transport.dma.sector(SLOT);
        let source = slot.get(..SECTOR_BYTES).ok_or(FatError::Device(sector))?;
        into.copy_from_slice(source);
        Ok(())
    }

    fn write(&mut self, sector: u32, from: &[u8; SECTOR]) -> Result<(), FatError> {
        if sector >= self.sectors {
            return Err(FatError::Device(sector));
        }
        let transport = self.inner.get_mut();
        transport.dma.sector(SLOT).copy_from_slice(from);
        transport.request(Kind::Out, sector)?;
        Ok(())
    }
}

/// Mounts the volume, formatting a disk that carries none.
///
/// The scratch disk arrives blank, so the format is what runs on the first
/// boot and the mount on every one after it.
fn mount<'a, 'b>(disk: &'a mut Disk<'b>) -> Result<FileSystem<&'a mut Disk<'b>>, Error> {
    match fs_fat::info(disk) {
        Ok(Some(_)) => FileSystem::mount(disk).map_err(server_fs::refusal),
        Ok(None) => FileSystem::format(disk, &FormatOptions::default()).map_err(server_fs::refusal),
        Err(error) => Err(server_fs::refusal(error)),
    }
}

/// Says what the volume is, which is what S4 of the plan ends with.
fn geometry(volume: &FileSystem<&mut Disk<'_>>, voice: Option<EndpointHandle>) {
    let shape = volume.geometry();
    let line = Report::of(format_args!(
        "[files] clusters={} free={} tables at {} of {} sectors\n",
        shape.clusters,
        volume.free_clusters(),
        shape.reserved_sectors,
        shape.fat_sectors
    ));
    report(volume.device(), voice, &line);
}

/// Answers clients until the endpoint fails.
fn serve(
    volume: &mut FileSystem<&mut Disk<'_>>,
    endpoint: EndpointHandle,
    voice: Option<EndpointHandle>,
) -> Result<(), Error> {
    let mut clients = Clients::new();
    let mut serving = Serving::default();
    loop {
        {
            let mut transport = volume.device().inner.borrow_mut();
            receive(&mut transport.gate, endpoint, &mut serving)?;
        }
        let decoded = {
            let transport = volume.device().inner.borrow();
            Request::decode(transport.gate.reader())
        };
        let now = {
            let mut transport = volume.device().inner.borrow_mut();
            transport
                .gate
                .clock_wall()
                .map_or_else(|_| moment(0), |(micros, _)| moment(micros))
        };
        // A message this server cannot read is a client that is not
        // speaking this protocol, and what it opened is dropped with it.
        let reply = if let Ok(request) = decoded {
            answer(volume, &mut clients, serving.badge, &request, now)
        } else {
            clients.forget(serving.badge);
            Reply::Flushed(Err(Error::InvalidArgument))
        };
        let mut transport = volume.device().inner.borrow_mut();
        if reply.encode(&mut transport.gate.writer()).is_err() {
            let _said =
                voice.map(|log| write_line(&mut transport.gate, log, b"[files] unsendable\n"));
        }
    }
}

/// The loop of a machine that carries no disk: every request is refused
/// and the server stays where it is, because a client that asks for a file
/// has to hear that there is none.
fn refuse_everything(gate: &mut Gate, endpoint: EndpointHandle) -> Result<(), Error> {
    let mut serving = Serving::default();
    loop {
        receive(gate, endpoint, &mut serving)?;
        let reply = Reply::Flushed(Err(Error::Unavailable));
        let _encoded = reply.encode(&mut gate.writer());
    }
}
