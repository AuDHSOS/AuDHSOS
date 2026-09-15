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
use audhsos_collections::ArrayVec;
use driver_virtio_blk::blk::{Blk, Vectors};
use driver_virtio_blk::request::{Kind, Request as BlkRequest, status};
use driver_virtio_blk::{BlkError, NO_VECTOR};
use fs_fat::{BlockDevice, Error as FatError, FileSystem, SECTOR};
use server_fs::{Clients, Partition, Volumes, answer, moment, mount};
use user_programs::client::{allocate, register, write_line};
use user_programs::dma::{Dma, QUEUE_SIZE, REGION_BYTES, SECTOR_BYTES};
use user_programs::mapping::Mapping;
use user_programs::registers::{BLOCK_WINDOW, Window};
use user_programs::serve::{Serving, receive};
use user_proto::file::{Reply, Request};
use user_rt::startup::{Device, MAX_BLOCK_DEVICES};
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

/// Brings the disks up, mounts what they carry, and answers clients.
///
/// Every window is mapped here and nowhere else: the drivers hold byte
/// slices over them for the rest of the program, and a borrow cannot
/// outlive the mapping it came from, so the mappings live in the frame
/// that never returns. The gate is shared rather than owned, because a
/// machine of two disks has two drivers that wait through it and
/// `Gate::adopt` allows one gate per buffer.
#[expect(
    clippy::needless_pass_by_value,
    reason = "the shape of `main` is what `program!` calls; the gate and the startup message belong to the program"
)]
fn main(gate: Gate, startup: Startup) -> ! {
    let voice = startup.log;
    let shared: Shared = RefCell::new(gate);
    let Some(endpoint) = startup.own_endpoint else {
        shared.borrow_mut().thread_exit()
    };
    if let Some(names) = startup.name_server {
        let mut open = shared.borrow_mut();
        let _registered = register(&mut open, names, NAME, endpoint);
    }
    let (Some(process), Some(memory)) = (startup.own_process, startup.memory_server) else {
        stop(&shared, voice, Error::AccessDenied)
    };

    let mut devices: ArrayVec<Given, MAX_BLOCK_DEVICES> = ArrayVec::new();
    for block in &startup.blocks {
        if let Some(given) = Given::of(block) {
            let _kept = devices.push(given);
        }
    }
    if devices.is_empty() {
        let line = Report::of(format_args!("[files] no disk\n"));
        say(&shared, voice, &line);
        let _refused = refuse_everything(&shared, endpoint);
        stop(&shared, voice, Error::Unavailable)
    }

    // One slot per device the machine may carry. They stand here, in the
    // frame that never returns, because what the drivers hold borrows
    // them.
    let mut slots: [Option<Windows>; MAX_BLOCK_DEVICES] = [None, None];
    for (index, (slot, device)) in slots.iter_mut().zip(devices.iter()).enumerate() {
        match map_windows(&shared, process, memory, device, index) {
            Ok(windows) => *slot = Some(windows),
            Err(error) => stop(&shared, voice, error),
        }
    }
    let [first, second] = &mut slots;
    let mut one = attach(&shared, first.as_mut(), devices.iter().next(), voice);
    let mut two = attach(&shared, second.as_mut(), devices.iter().nth(1), voice);

    // Which disk is which is what is on them and not the order the bus
    // has them: a disk that carries a partition table is one the firmware
    // wrote and the system reads.
    let (booted, written) = sort(&mut one, &mut two);
    let mut booted = booted.map(mount);
    let mut written = written.map(mount);
    say_volumes(&shared, voice, booted.as_ref(), written.as_ref());

    let outcome = serve(
        &shared,
        Volumes {
            written: written.as_mut().and_then(|volume| volume.as_mut().ok()),
            booted: booted.as_mut().and_then(|volume| volume.as_mut().ok()),
        },
        endpoint,
        voice,
    );
    if let Err(error) = outcome {
        let line = Report::of(format_args!("[files] {}\n", error.message()));
        say(&shared, voice, &line);
    }
    shared.borrow_mut().thread_exit()
}

/// The disk of one slot, brought up, or nothing where the machine carries
/// no such device or the device would not come up.
fn attach<'a>(
    shared: &'a Shared,
    windows: Option<&'a mut Windows>,
    device: Option<&Given>,
    voice: Option<EndpointHandle>,
) -> Option<Disk<'a>> {
    let (windows, device) = (windows?, device?);
    match bring_up_disk(shared, windows, device, voice) {
        Ok(disk) => Some(disk),
        Err(error) => {
            let line = Report::of(format_args!("[files] a disk: {}\n", error.message()));
            say(shared, voice, &line);
            None
        }
    }
}

/// The two disks as the volumes they carry: the one somebody partitioned,
/// and the one this system may write.
fn sort<'a, 'b>(
    one: &'a mut Option<Disk<'b>>,
    two: &'a mut Option<Disk<'b>>,
) -> (Option<&'a mut Disk<'b>>, Option<&'a mut Disk<'b>>) {
    let first_is_booted = one.as_ref().is_some_and(server_fs::is_partitioned);
    let second_is_booted = two.as_ref().is_some_and(server_fs::is_partitioned);
    match (first_is_booted, second_is_booted) {
        // Two disks that both carry a table are two disks somebody else
        // wrote, and the machine has nothing it may write.
        (true, true) => (one.as_mut(), None),
        (true, false) => (one.as_mut(), two.as_mut()),
        (false, true) => (two.as_mut(), one.as_mut()),
        (false, false) => (None, one.as_mut().or(two.as_mut())),
    }
}

/// Maps the two windows of one device and brings the device up.
fn bring_up_disk<'a>(
    shared: &'a Shared,
    windows: &'a mut Windows,
    device: &Given,
    voice: Option<EndpointHandle>,
) -> Result<Disk<'a>, Error> {
    let physical = windows.physical;
    // SAFETY: both mappings stand for the rest of this program — neither
    // is unmapped and `main` does not return — each is memory of this
    // process alone, and the two values built here are the only ones that
    // reach those bytes.
    let mut registers = Window::new(unsafe { windows.registers.bytes() }, device.places);
    // SAFETY: as above.
    let mut dma = Dma::new(unsafe { windows.region.bytes() }, physical).ok_or(Error::Unaligned)?;
    let (blk, queue, sectors) = {
        let mut gate = shared.borrow_mut();
        bring_up(&mut gate, &mut dma, &mut registers, device)?
    };
    Ok(Disk {
        sectors,
        inner: RefCell::new(Transport {
            gate: shared,
            registers,
            dma,
            blk,
            queue,
            notification: device.notification,
            interrupt: device.interrupt,
            bit: device.bit,
            voice,
        }),
    })
}

/// Says why the server stops and ends the thread.
fn stop(shared: &Shared, voice: Option<EndpointHandle>, error: Error) -> ! {
    let line = Report::of(format_args!("[files] {}\n", error.message()));
    say(shared, voice, &line);
    shared.borrow_mut().thread_exit()
}

/// Writes one line, if there is anywhere to write it.
fn say(shared: &Shared, voice: Option<EndpointHandle>, line: &Report) {
    let (Some(console), Ok(mut gate)) = (voice, shared.try_borrow_mut()) else {
        return;
    };
    let _logged = write_line(&mut gate, console, line.as_bytes());
}

/// Says what each volume is, which is what S4 of the plan ends with.
fn say_volumes<'a>(
    shared: &Shared,
    voice: Option<EndpointHandle>,
    booted: Option<&Result<Volume<'a>, Error>>,
    written: Option<&Result<Volume<'a>, Error>>,
) {
    for (name, volume) in [("boot", booted), ("scratch", written)] {
        let line = match volume {
            Some(Ok(volume)) => {
                let shape = volume.geometry();
                Report::of(format_args!(
                    "[files] {name} volume: clusters={} free={} tables at {} of {} sectors\n",
                    shape.clusters,
                    volume.free_clusters(),
                    shape.reserved_sectors,
                    shape.fat_sectors
                ))
            }
            Some(Err(error)) => Report::of(format_args!(
                "[files] no {name} volume: {}\n",
                error.message()
            )),
            None => Report::of(format_args!("[files] no {name} disk\n")),
        };
        say(shared, voice, &line);
    }
}

/// The volume one disk carries.
type Volume<'a> = FileSystem<Partition<&'a mut Disk<'a>>>;

/// What the root task gave this process about one device.
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
    /// What the nine roles of one device say, or nothing when they say it
    /// only in part.
    fn of(block: &Device) -> Option<Self> {
        let places = block.structures()?;
        let mut end = 0u64;
        for place in &places {
            end = end.max(u64::from(place.offset).wrapping_add(u64::from(place.len)));
        }
        Some(Given {
            registers: block.registers,
            bytes: end.next_multiple_of(PAGE_SIZE),
            places,
            multiplier: u32::try_from(block.notify_multiplier?).ok()?,
            notification: block.notification?,
            interrupt: block.interrupt?,
            bit: block.vector_bit?,
        })
    }
}

/// The two mappings one driver works through.
struct Windows {
    registers: Mapping,
    region: Mapping,
    physical: u64,
}

/// Maps the register window of one device and the region it reads and
/// writes.
fn map_windows(
    shared: &Shared,
    process: ProcessHandle,
    memory: EndpointHandle,
    device: &Given,
    index: usize,
) -> Result<Windows, Error> {
    let mut gate = shared.borrow_mut();
    let registers = Mapping::new(
        &mut gate,
        process,
        device.registers,
        window_at(index),
        device.bytes,
    )?;
    // What is asked for and what is mapped are one number, so a larger
    // queue or a second slot needs no change here.
    let bytes = u64::try_from(REGION_BYTES)
        .unwrap_or(0)
        .next_multiple_of(PAGE_SIZE);
    let object = allocate(&mut gate, memory, bytes, PAGE_SIZE)?;
    let physical = gate.memory_info(object)?.start;
    let region = Mapping::new(&mut gate, process, object, region_at(index), bytes)?;
    Ok(Windows {
        registers,
        region,
        physical,
    })
}

/// Where the register window of the device at `index` is mapped.
///
/// Each device is given a window of the address space of its own. The
/// index is what keeps them apart and not the bit of the notification:
/// every device is given a notification of its own, so every one of them
/// carries the first bit.
fn window_at(index: usize) -> u64 {
    BLOCK_WINDOW.wrapping_add(apart(index))
}

/// Where the region the device at `index` reads and writes is mapped.
fn region_at(index: usize) -> u64 {
    DMA.wrapping_add(apart(index))
}

/// How far the windows of the device at `index` stand from the first.
fn apart(index: usize) -> u64 {
    u64::try_from(index).unwrap_or(0).wrapping_mul(APART)
}

/// How far apart the windows of two devices are mapped.
const APART: u64 = 0x0000_0100_0000_0000;

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
    gate: &'a RefCell<Gate>,
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

/// The one gate of this thread, which the loop and every driver reach.
///
/// `Gate::adopt` allows one gate per IPC buffer, and a machine of two
/// disks has two drivers that wait through it, so it is shared and not
/// owned. No borrow of it is held across a call into `fs-fat`.
type Shared = RefCell<Gate>;

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
            .borrow_mut()
            .clock_now()
            .map_err(|_| FatError::Device(sector))?
            .saturating_add(DEADLINE);
        let mask = 1u64.wrapping_shl(u32::try_from(self.bit).unwrap_or(0));
        loop {
            if self
                .gate
                .borrow_mut()
                .clock_now()
                .is_ok_and(|now| now >= deadline)
            {
                return Err(self.give_up(sector));
            }
            let bits = {
                let mut gate = self.gate.borrow_mut();
                let bits = gate.notification_wait_until(self.notification, deadline);
                let _acked = gate.interrupt_ack(self.interrupt);
                bits
            };
            let Ok(bits) = bits else {
                return Err(self.give_up(sector));
            };
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
    fn trace(&self, what: core::fmt::Arguments<'_>) {
        let Some(console) = self.voice else {
            return;
        };
        let Ok(mut gate) = self.gate.try_borrow_mut() else {
            return;
        };
        let line = Report::of(what);
        let _said = write_line(&mut gate, console, line.as_bytes());
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

/// Answers clients until the endpoint fails.
fn serve(
    shared: &Shared,
    mut volumes: Volumes<'_, Partition<&mut Disk<'_>>>,
    endpoint: EndpointHandle,
    voice: Option<EndpointHandle>,
) -> Result<(), Error> {
    let mut clients = Clients::new();
    let mut serving = Serving::default();
    loop {
        {
            let mut gate = shared.borrow_mut();
            receive(&mut gate, endpoint, &mut serving)?;
        }
        let (decoded, now) = {
            let mut gate = shared.borrow_mut();
            let decoded = Request::decode(gate.reader());
            let now = gate
                .clock_wall()
                .map_or_else(|_| moment(0), |(micros, _)| moment(micros));
            (decoded, now)
        };
        // A message this server cannot read is a client that is not
        // speaking this protocol, and what it opened is dropped with it.
        let reply = if let Ok(request) = decoded {
            answer(&mut volumes, &mut clients, serving.badge, &request, now)
        } else {
            clients.forget(serving.badge);
            Reply::Flushed(Err(Error::InvalidArgument))
        };
        let encoded = {
            let mut gate = shared.borrow_mut();
            reply.encode(&mut gate.writer())
        };
        if encoded.is_err() {
            let line = Report::of(format_args!("[files] unsendable\n"));
            say(shared, voice, &line);
        }
    }
}

/// The loop of a machine that carries no disk: every request is refused
/// and the server stays where it is, because a client that asks for a file
/// has to hear that there is none.
fn refuse_everything(shared: &Shared, endpoint: EndpointHandle) -> Result<(), Error> {
    let mut serving = Serving::default();
    loop {
        let mut gate = shared.borrow_mut();
        receive(&mut gate, endpoint, &mut serving)?;
        let reply = Reply::Flushed(Err(Error::Unavailable));
        let _encoded = reply.encode(&mut gate.writer());
    }
}
