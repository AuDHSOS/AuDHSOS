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
use user_proto::parent;
use virtio_queue as _;

use audhsos_abi::layout::{MAX_MESSAGE_HANDLES, PAGE_SIZE};
use audhsos_abi::startup::{BusRange, Location, Payload, Role, Screen, Writer};
use audhsos_abi::{Error, Handle, Rights};
use pci::address::{Address, BYTES_PER_BUS, Window};
use pci::bar::{Bar, Space as BarSpace, probe};
use pci::capability::{ID_MSIX, find, walk};
use pci::enumerate::walk as enumerate;
use pci::error::PciError;
use pci::header::{COMMAND_BUS_MASTER, COMMAND_MEMORY, read_command, write_command};
use pci::msix;
use pci::virtio::{self, BLOCK_DEVICE, VIRTIO_VENDOR};
use user_loader::tar::{Archive, Kind};
use user_loader::{Plan, STACK_PAGES};
use user_programs::client::allocate;
use user_programs::config_space::MappedSpace;
use user_programs::mapping::{Mapping, SCRATCH};
use user_programs::{permissions, priority};
use user_rt::{
    EndpointHandle, InterruptHandle, IoPortHandle, Line, MemoryHandle, NotificationHandle,
    ProcessHandle, Startup, SystemControlHandle, Typed,
};
use user_sys_x86_64::gate::MessageInterrupt;
use user_sys_x86_64::{self as sys, Gate, Mmio};

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
    /// The framebuffer of the machine and the mode the firmware set.
    Framebuffer,
    /// The ports of the PS/2 controller and the two lines its devices
    /// interrupt on.
    Input,
    /// The configuration window of the PCI bus and the buses it covers.
    Ecam,
    /// The registers of the virtio block device, its message interrupt,
    /// and the notification that interrupt is bound to.
    Block,
}

/// One line of the start table: what to start, how, and with what.
#[expect(
    clippy::struct_excessive_bools,
    reason = "one flag per thing a program may reach, which is what a table of who gets what is"
)]
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
    /// Whether it may draw, which is a badged capability to the display
    /// server. A program that finds the server by name instead is refused
    /// by it: an unbadged request names nobody.
    draws: bool,
    /// Whether it may listen, which is a badged capability to the input
    /// server, for the same reason.
    listens: bool,
    /// Whether it reports to this program when it is done. The machine
    /// ends when every program that reports has reported.
    reports: bool,
}

/// The programs of this system, in the order they are started.
///
/// The quotas are what the programs measured out at need with room over
/// them; a program that asks for more than its line says is refused by the
/// kernel and not by this table.
const PROGRAMS: [Program; 14] = [
    Program {
        name: b"server-memory",
        priority: priority::SERVER,
        handles: 512,
        frames: 512,
        objects: 1024,
        grant: Grant::Ram,
        names: false,
        memory: false,
        draws: false,
        listens: false,
        reports: false,
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
        draws: false,
        listens: false,
        reports: false,
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
        draws: false,
        listens: false,
        reports: false,
    },
    // The display server maps the framebuffer, which is four mebibytes on
    // the reference machine, and one surface per client beside it, so its
    // quota of frames is the largest of any program here.
    Program {
        name: b"server-display",
        priority: priority::DRIVER,
        handles: 64,
        frames: 256,
        objects: 64,
        grant: Grant::Framebuffer,
        names: true,
        memory: true,
        draws: false,
        listens: false,
        reports: false,
    },
    // The file system server drives the block device: the register
    // window, the message interrupt, and the notification that interrupt
    // is bound to. It maps the window and one page the device reads and
    // writes, so its quota of frames is a driver's and not an
    // application's. A machine without the device starts it all the same,
    // and it answers that there is no disk.
    Program {
        name: b"server-fs",
        priority: priority::DRIVER,
        handles: 64,
        frames: 64,
        objects: 64,
        grant: Grant::Block,
        names: true,
        memory: true,
        draws: false,
        listens: false,
        reports: false,
    },
    // The input server owns the PS/2 controller and both of its lines. It
    // starts after the display server so that a machine which has both
    // brings the screen up before anything is typed at it.
    Program {
        name: b"server-input",
        priority: priority::DRIVER,
        handles: 64,
        frames: 64,
        objects: 64,
        grant: Grant::Input,
        names: true,
        memory: true,
        draws: false,
        listens: false,
        reports: false,
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
        draws: false,
        listens: false,
        reports: true,
    },
    Program {
        name: b"app-checks",
        priority: priority::APPLICATION,
        handles: 32,
        frames: 32,
        objects: 32,
        grant: Grant::None,
        names: true,
        memory: true,
        draws: false,
        listens: true,
        reports: true,
    },
    Program {
        name: b"app-paint",
        priority: priority::APPLICATION,
        handles: 32,
        frames: 256,
        objects: 32,
        grant: Grant::None,
        names: true,
        memory: true,
        draws: true,
        listens: false,
        reports: true,
    },
    Program {
        name: b"app-input",
        priority: priority::APPLICATION,
        handles: 32,
        frames: 32,
        objects: 32,
        grant: Grant::None,
        names: true,
        memory: true,
        draws: false,
        listens: true,
        reports: true,
    },
    // The canvas draws and listens at once, and its surface is the size of
    // the screen, so it needs the frames the program that paints needs and
    // both badged capabilities.
    Program {
        name: b"app-canvas",
        priority: priority::APPLICATION,
        handles: 32,
        frames: 256,
        objects: 32,
        grant: Grant::None,
        names: true,
        memory: true,
        draws: true,
        listens: true,
        reports: true,
    },
    // The bus walk maps one mebibyte of the configuration window at a time,
    // which is a page table and the levels above it, so its quota of frames
    // is the smallest of any program that maps anything.
    Program {
        name: b"app-lspci",
        priority: priority::APPLICATION,
        handles: 32,
        frames: 32,
        objects: 32,
        grant: Grant::Ecam,
        names: true,
        memory: true,
        draws: false,
        listens: false,
        reports: true,
    },
    // The program that uses the file system server. It starts after the
    // bus walk so that the lines of the two do not interleave.
    Program {
        name: b"app-files",
        priority: priority::APPLICATION,
        handles: 32,
        frames: 32,
        objects: 32,
        grant: Grant::None,
        names: true,
        memory: true,
        draws: false,
        listens: false,
        reports: true,
    },
    // It faults and its thread stops there, so it never reports and the
    // machine does not wait for it.
    Program {
        name: b"app-faulter",
        priority: priority::APPLICATION,
        handles: 32,
        frames: 32,
        objects: 32,
        grant: Grant::None,
        names: true,
        memory: true,
        draws: false,
        listens: false,
        reports: false,
    },
];

/// How many programs of the table report when they are done.
fn reporters() -> usize {
    PROGRAMS.iter().filter(|program| program.reports).count()
}

/// The port of the exit device of the machine and how many there are.
const EXIT_PORT: u64 = 0xF4;
const EXIT_PORTS: u64 = 4;

/// The value a write to the exit device reports a success with.
const EXIT_SUCCESS: u64 = 0x10;

/// The first port of the serial controller and how many there are.
const COM1: u64 = 0x3F8;
const COM1_PORTS: u64 = 8;

/// The interrupt line the controller raises on.
const COM1_LINE: u64 = 4;

/// The first port of the PS/2 controller and how many there are: the data
/// register at `0x60` and the command register at `0x64`, with the four
/// ports between them inside the range because a program is given one.
const PS2_PORT: u64 = 0x60;
const PS2_PORTS: u64 = 5;

/// The line the keyboard asserts on, and the line the mouse asserts on.
const PS2_KEYBOARD_LINE: u64 = 1;
const PS2_MOUSE_LINE: u64 = 12;

/// The badge the root task's own messages to a server carry.
///
/// A server tells its clients apart by the badge of the capability the
/// message came through, and a capability without one names nobody: the
/// memory server refuses such a request outright, because memory it handed
/// to nobody is memory it could never take back.
const INIT_BADGE: u64 = u64::MAX;

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
        memory_for_self: None,
        console: None,
        display: None,
        input: None,
        console_for_self: None,
        ecam: None,
        block: None,
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
    serve_faults(&mut gate, &world, reporters())
}

/// What the root task knows about the system it has started.
struct World {
    own: ProcessHandle,
    faults: EndpointHandle,
    system: Option<SystemControlHandle>,
    /// The endpoint of the name server, as the root task holds it.
    names: Option<EndpointHandle>,
    /// The endpoint of the memory server.
    memory: Option<EndpointHandle>,
    /// The same, badged for the root task's own requests.
    memory_for_self: Option<EndpointHandle>,
    /// The endpoint of the console driver.
    console: Option<EndpointHandle>,
    /// The endpoint of the display server.
    display: Option<EndpointHandle>,
    /// The endpoint of the input server.
    input: Option<EndpointHandle>,
    /// The same, badged for the root task's own lines.
    console_for_self: Option<EndpointHandle>,
    /// The configuration window of the PCI bus, made once: the root task
    /// enumerates through it and `app-lspci` is given a handle to the same
    /// object.
    ecam: Option<(MemoryHandle, BusRange)>,
    /// The virtio block device, as the enumeration found and prepared it,
    /// or nothing for a machine that carries none.
    block: Option<Block>,
    /// What the next child reports its faults under.
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
) -> Result<(), Failure> {
    let plan = user_loader::plan(elf).map_err(|_| Failure {
        step: "elf",
        error: Error::InvalidArgument,
    })?;
    let child = gate
        .process_create(world.own, program.handles, program.frames, program.objects)
        .map_err(at("process"))?;
    let endpoint = gate.endpoint_create().map_err(at("endpoint"))?;

    for region in plan.regions() {
        let object = take(gate, world, reserve, region.len, PAGE_SIZE).map_err(at("region"))?;
        copy_region(gate, world.own, object, &region, elf).map_err(at("copy"))?;
        let bits = if region.write {
            permissions::WRITE
        } else if region.execute {
            permissions::EXECUTE
        } else {
            permissions::READ
        };
        map_into(gate, child, object, region.vaddr, region.len, bits).map_err(at("map"))?;
    }

    let stack_bytes = STACK_PAGES.wrapping_mul(PAGE_SIZE);
    let stack = take(gate, world, reserve, stack_bytes, PAGE_SIZE).map_err(at("stack"))?;
    zero(gate, world.own, stack, stack_bytes).map_err(at("stack zero"))?;
    map_into(
        gate,
        child,
        stack,
        plan.stack_base,
        stack_bytes,
        permissions::WRITE,
    )
    .map_err(at("stack map"))?;

    let buffer = take(gate, world, reserve, PAGE_SIZE, PAGE_SIZE).map_err(at("buffer"))?;
    let badge = world.badge();
    install_all(
        gate, startup, world, program, child, endpoint, badge, buffer,
    )
    .map_err(at("handles"))?;

    let handler = badged(gate, world.faults, badge).map_err(at("handler"))?;
    gate.process_set_fault_handler(child, Some(handler))
        .map_err(at("fault handler"))?;
    let thread = gate
        .thread_create(
            child,
            plan.entry,
            plan.stack_top,
            u64::from(program.priority),
            u64::from(program.priority),
            Some(buffer),
        )
        .map_err(at("thread"))?;
    gate.thread_start(thread).map_err(at("start"))?;
    remember(gate, world, program, endpoint).map_err(at("remember"))
}

/// Names the step an error came out of, so that a line the root task writes
/// says where the boot stopped and not only that it did.
const fn at(step: &'static str) -> impl Fn(Error) -> Failure {
    move |error| Failure { step, error }
}

/// A step that did not work.
#[derive(Clone, Copy, Debug)]
struct Failure {
    step: &'static str,
    error: Error,
}

/// Installs everything the child gets and writes the startup message into
/// the page that will be its IPC buffer.
#[expect(
    clippy::too_many_arguments,
    reason = "what a child is given is what a child is given; a struct for it would be the same list with a name"
)]
fn install_all(
    gate: &mut Gate,
    startup: &Startup,
    world: &mut World,
    program: &Program,
    child: ProcessHandle,
    endpoint: EndpointHandle,
    badge: u64,
    buffer: MemoryHandle,
) -> Result<(), Error> {
    let mut mapping = Mapping::new(gate, world.own, buffer, SCRATCH, PAGE_SIZE)?;
    // SAFETY: the page is mapped and nothing else holds a reference to it.
    unsafe {
        mapping.zero();
    }
    let mut given: [(Role, Payload); MAX_GIVEN] =
        [(Role::OwnProcess, Payload::Handle(Handle::MAX)); MAX_GIVEN];
    let mut count = 0usize;

    let own = gate.process_install_handle(child, child.handle(), ObjectRights::PROCESS)?;
    push(&mut given, &mut count, Role::OwnProcess, own)?;
    let served = gate.process_install_handle(child, endpoint.handle(), ObjectRights::RECV)?;
    push(&mut given, &mut count, Role::OwnEndpoint, served)?;

    // What a child holds of a server is a capability with the child's badge
    // on it: the badge is the only thing about a sender a server can trust,
    // and a request that arrives without one names nobody.
    if program.names
        && let Some(names) = world.names
    {
        let marked = gate.endpoint_badge(names, badge)?;
        let handle = gate.process_install_handle(child, marked.handle(), ObjectRights::SEND)?;
        push(&mut given, &mut count, Role::NameServer, handle)?;
    }
    if program.memory
        && let Some(memory) = world.memory
    {
        let marked = gate.endpoint_badge(memory, badge)?;
        let handle = gate.process_install_handle(child, marked.handle(), ObjectRights::SEND)?;
        push(&mut given, &mut count, Role::MemoryServer, handle)?;
    }
    // A program that draws is known to the display server by the badge its
    // parent puts on: a capability found under a name carries none, and a
    // server that keeps something per client cannot tell two of those apart.
    if program.draws
        && let Some(display) = world.display
    {
        let marked = gate.endpoint_badge(display, badge)?;
        let handle = gate.process_install_handle(child, marked.handle(), ObjectRights::SEND)?;
        push(&mut given, &mut count, Role::DisplayServer, handle)?;
    }
    // A program that listens is known to the input server the same way, and
    // for the same reason: the server keeps a ring per client.
    if program.listens
        && let Some(input) = world.input
    {
        let marked = gate.endpoint_badge(input, badge)?;
        let handle = gate.process_install_handle(child, marked.handle(), ObjectRights::SEND)?;
        push(&mut given, &mut count, Role::InputServer, handle)?;
    }
    // Everyone but the console driver gets the console as its log. The
    // driver is the console: a line it sent itself would be a call on the
    // endpoint it is the only receiver of, and it would wait for itself.
    // A program that reports gets the endpoint the kernel sends its faults
    // to, under the badge its parent knows it by: one endpoint carries both
    // kinds of news about a child, told apart by the label.
    if program.reports {
        let marked = gate.endpoint_badge(world.faults, badge)?;
        let handle = gate.process_install_handle(child, marked.handle(), ObjectRights::SEND)?;
        push(&mut given, &mut count, Role::Parent, handle)?;
    }
    if let Some(console) = world.console
        && program.grant != Grant::Serial
    {
        let marked = gate.endpoint_badge(console, badge)?;
        let handle = gate.process_install_handle(child, marked.handle(), ObjectRights::SEND)?;
        push(&mut given, &mut count, Role::Log, handle)?;
    }
    grant(gate, startup, world, program, child, &mut given, &mut count)?;

    let mut writer = Writer::new();
    {
        // SAFETY: as above.
        let bytes = unsafe { mapping.bytes() };
        let Some(page) = bytes.first_chunk_mut::<{ audhsos_abi::ipc_buffer::SIZE }>() else {
            return Err(Error::BufferTooSmall);
        };
        let mut buffer = audhsos_abi::BufferMut::new(page);
        for (role, payload) in given.iter().take(count) {
            match payload {
                Payload::Handle(handle) => writer.give(&mut buffer, *role, *handle)?,
                Payload::Value(value) => writer.tell(&mut buffer, *role, *value)?,
            }
        }
        writer.finish(&mut buffer)?;
    }
    mapping.unmap(gate, world.own)
}

/// Installs what a program's line of the table grants it beyond the
/// endpoints every program gets.
fn grant(
    gate: &mut Gate,
    startup: &Startup,
    world: &mut World,
    program: &Program,
    child: ProcessHandle,
    given: &mut [(Role, Payload); MAX_GIVEN],
    count: &mut usize,
) -> Result<(), Error> {
    match program.grant {
        Grant::None => {}
        Grant::Ram => {
            for region in startup.ram.iter().skip(1) {
                let handle =
                    gate.process_install_handle(child, region.handle(), ObjectRights::MEMORY)?;
                push(given, count, Role::Ram, handle)?;
            }
        }
        Grant::Serial => {
            let (ports, line) = serial(gate, world)?;
            let handle = gate.process_install_handle(child, ports.handle(), ObjectRights::PORTS)?;
            push(given, count, Role::IoPorts, handle)?;
            let handle = gate.process_install_handle(child, line.handle(), ObjectRights::MANAGE)?;
            push(given, count, Role::Interrupt, handle)?;
        }
        // A machine without a framebuffer grants nothing here: the display
        // server starts all the same and answers that there is no screen.
        Grant::Framebuffer => {
            if let Some((memory, screen)) = framebuffer(gate, world)? {
                let handle =
                    gate.process_install_handle(child, memory.handle(), ObjectRights::DEVICE)?;
                push(given, count, Role::Framebuffer, handle)?;
                tell(given, count, Role::FramebufferGeometry, screen.geometry())?;
                tell(given, count, Role::FramebufferLine, screen.line())?;
            }
        }
        // A machine whose firmware published no window grants nothing here:
        // the program starts all the same and reports that there is none.
        Grant::Ecam => {
            if let Some((memory, buses)) = window(gate, world) {
                let handle =
                    gate.process_install_handle(child, memory.handle(), ObjectRights::DEVICE)?;
                push(given, count, Role::Ecam, handle)?;
                tell(given, count, Role::EcamBuses, buses.word())?;
            }
        }
        // A machine that carries no virtio block device grants nothing
        // here, for the reason a machine without a window grants nothing
        // above: the program starts and reports that it was given none.
        Grant::Block => {
            if let Some(block) = block_device(gate, world) {
                let handle = gate.process_install_handle(
                    child,
                    block.registers.handle(),
                    ObjectRights::DEVICE,
                )?;
                push(given, count, Role::BlockRegisters, handle)?;
                tell(given, count, Role::BlockCommon, block.places[0].word())?;
                tell(given, count, Role::BlockNotify, block.places[1].word())?;
                tell(given, count, Role::BlockIsr, block.places[2].word())?;
                tell(given, count, Role::BlockConfig, block.places[3].word())?;
                tell(
                    given,
                    count,
                    Role::BlockNotifyMultiplier,
                    u64::from(block.multiplier),
                )?;
                let handle = gate.process_install_handle(
                    child,
                    block.interrupt.handle(),
                    ObjectRights::MANAGE,
                )?;
                push(given, count, Role::BlockInterrupt, handle)?;
                let handle = gate.process_install_handle(
                    child,
                    block.notification.handle(),
                    ObjectRights::NOTIFY,
                )?;
                push(given, count, Role::BlockNotification, handle)?;
                tell(given, count, Role::BlockVectorBit, BLOCK_VECTOR_BIT)?;
            }
        }
        Grant::Input => {
            let (ports, keyboard, mouse) = ps2(gate, world)?;
            let handle = gate.process_install_handle(child, ports.handle(), ObjectRights::PORTS)?;
            push(given, count, Role::IoPorts, handle)?;
            let handle =
                gate.process_install_handle(child, keyboard.handle(), ObjectRights::MANAGE)?;
            push(given, count, Role::Interrupt, handle)?;
            let handle =
                gate.process_install_handle(child, mouse.handle(), ObjectRights::MANAGE)?;
            push(given, count, Role::AuxInterrupt, handle)?;
        }
    }
    Ok(())
}

/// How many pairs the startup message holds: it is a run of role and
/// handle over the message area, so half the words of one.
const MAX_GIVEN: usize = audhsos_abi::layout::MAX_MESSAGE_WORDS / 2;

/// Records one pair of the startup message.
///
/// A pair that does not fit is refused rather than dropped: a child that
/// silently received fewer capabilities than its parent meant to give it
/// would fail somewhere else, for a reason nothing names.
fn push(
    given: &mut [(Role, Payload); MAX_GIVEN],
    count: &mut usize,
    role: Role,
    handle: Handle,
) -> Result<(), Error> {
    record(given, count, role, Payload::Handle(handle))
}

/// Records one pair that carries a number instead of a handle (D-103).
fn tell(
    given: &mut [(Role, Payload); MAX_GIVEN],
    count: &mut usize,
    role: Role,
    value: u64,
) -> Result<(), Error> {
    record(given, count, role, Payload::Value(value))
}

/// Records one pair, whatever its second word means.
fn record(
    given: &mut [(Role, Payload); MAX_GIVEN],
    count: &mut usize,
    role: Role,
    payload: Payload,
) -> Result<(), Error> {
    let slot = given.get_mut(*count).ok_or(Error::BufferTooSmall)?;
    *slot = (role, payload);
    *count = count.wrapping_add(1);
    Ok(())
}

/// The rights each kind of handle goes out with.
struct ObjectRights;

impl ObjectRights {
    /// A program's own process: it maps memory into itself, manages its
    /// own threads, and may hand a capability to itself to a server that
    /// gives something back when it ends — which is `INFO` and, to be able
    /// to give it away at all, `DUPLICATE` and `TRANSFER` (D-106).
    const PROCESS: Rights = Rights::MAP
        .union(Rights::MANAGE)
        .union(Rights::INFO)
        .union(Rights::DUPLICATE)
        .union(Rights::TRANSFER);
    /// A program's own endpoint: it receives on it, badges it for the
    /// clients it hands it to, and passes it on — which is what registering
    /// a name is, and what needs `TRANSFER`.
    const RECV: Rights = Rights::RECV
        .union(Rights::BADGE)
        .union(Rights::SEND)
        .union(Rights::TRANSFER)
        .union(Rights::DUPLICATE);
    const SEND: Rights = Rights::SEND.union(Rights::TRANSFER);
    /// A memory object goes out with every right its type accepts but the
    /// duplication of the handle. `EXECUTE` is among them and has to be:
    /// memory the server hands back becomes the text of some program, and
    /// an object without the right cannot be mapped executable.
    const MEMORY: Rights = Rights::READ
        .union(Rights::WRITE)
        .union(Rights::EXECUTE)
        .union(Rights::MAP)
        .union(Rights::INFO)
        .union(Rights::TRANSFER);
    const PORTS: Rights = Rights::READ.union(Rights::WRITE);
    /// The framebuffer: it is written to and mapped, and it is not code.
    const DEVICE: Rights = Rights::READ
        .union(Rights::WRITE)
        .union(Rights::MAP)
        .union(Rights::INFO);
    const MANAGE: Rights = Rights::MANAGE;
    /// A notification a driver was given: it waits on it, and what sets
    /// the bit is the interrupt the root task bound to it. The driver
    /// signals nothing and binds nothing, so it is given neither right.
    const NOTIFY: Rights = Rights::WAIT;
}

/// The framebuffer of the machine as a memory object, with the mode the
/// firmware set, or nothing when the machine has no framebuffer.
///
/// `system_info` is what says where it is: the kernel keeps the description
/// the loader left and reports it, and this is the only program that may
/// ask, because the object is made with the root authority.
fn framebuffer(gate: &mut Gate, world: &World) -> Result<Option<(MemoryHandle, Screen)>, Error> {
    let system = world.system.ok_or(Error::AccessDenied)?;
    let Some(described) = gate.system_info(system)?.framebuffer else {
        return Ok(None);
    };
    let first = described.phys_start.wrapping_div(PAGE_SIZE);
    let frames = described.len.wrapping_div(PAGE_SIZE);
    let memory = gate.memory_create_device(system, first, frames)?;
    Ok(Some((
        memory,
        Screen {
            width: described.width,
            height: described.height,
            stride: described.stride,
            format: described.format,
        },
    )))
}

/// The configuration window of the PCI bus as a memory object, with the
/// buses it covers, or nothing when the firmware published no `MCFG` table.
///
/// `system_info` is what says where it is, for the reason it says where the
/// framebuffer is: the kernel read the table at bring-up and reports what it
/// found, and this is the only program that may ask.
fn ecam(gate: &mut Gate, world: &World) -> Result<Option<(MemoryHandle, BusRange)>, Error> {
    let system = world.system.ok_or(Error::AccessDenied)?;
    let Some(window) = gate.system_info(system)?.ecam else {
        return Ok(None);
    };
    let first = window.base.wrapping_div(PAGE_SIZE);
    let frames = window.len().wrapping_div(PAGE_SIZE);
    // A window the kernel will not make an object of — one that meets
    // memory the machine reported as usable, or one the address space
    // cannot hold — is no reason to leave the program unstarted. It starts
    // without the window and reports that there is none; a program of the
    // table that never starts is a report that never comes, and the machine
    // waits for it.
    let Ok(memory) = gate.memory_create_device(system, first, frames) else {
        return Ok(None);
    };
    Ok(Some((
        memory,
        BusRange {
            segment: window.segment,
            first_bus: window.first_bus,
            last_bus: window.last_bus,
        },
    )))
}

/// Names the step a refusal of the handover came out of, so that a line
/// the root task writes says where the handover stopped and not only that
/// it did.
trait Step<T> {
    /// The value, or the refusal under the name of `step`.
    fn step(self, step: &'static str) -> Result<T, Refused>;
}

impl<T> Step<T> for Result<T, Error> {
    fn step(self, step: &'static str) -> Result<T, Refused> {
        self.map_err(|error| Refused {
            step,
            error,
            about: None,
        })
    }
}

/// A step of the handover that did not work, and the physical range it
/// was about where there is one.
#[derive(Clone, Copy, Debug)]
struct Refused {
    step: &'static str,
    error: Error,
    about: Option<(u64, u64)>,
}

impl From<Refused> for Error {
    fn from(refused: Refused) -> Error {
        refused.error
    }
}

/// The virtio block device of the machine, as the root task found it and
/// made it ready to be driven.
struct Block {
    /// Device memory over the base address register the four structures
    /// lie in, aligned outward to whole frames.
    registers: MemoryHandle,
    /// Where each structure lies in that window, in the order
    /// [`STRUCTURES`] names. The offsets count from the start of the
    /// window and not from the base address register, so a register that
    /// does not start at a frame is no special case for the driver.
    places: [Location; 4],
    /// The multiplier of virtio 4.1.4.4.
    multiplier: u32,
    /// The message interrupt of the device.
    interrupt: InterruptHandle,
    /// The notification that interrupt is bound to.
    notification: NotificationHandle,
    /// What the message table of the device says once the entry is
    /// written: where the table is, whether the function raises MSI-X,
    /// and the write a completion makes.
    table: msix::MsiX,
    /// What the entry of the message table reads back as, once it is
    /// written.
    back: msix::Entry,
}

/// The four structures a driver of this system needs, in the order the
/// four value roles carry them.
const STRUCTURES: [virtio::Kind; 4] = [
    virtio::Kind::Common,
    virtio::Kind::Notify,
    virtio::Kind::Isr,
    virtio::Kind::Device,
];

/// The bit of the notification the message interrupt of the block device
/// sets. The notification carries this device alone, so it is the first.
const BLOCK_VECTOR_BIT: u64 = 0;

/// The entry of the MSI-X table the vector is written into. The driver
/// uses one vector, so it is the first.
const BLOCK_MSIX_ENTRY: u32 = 0;

/// The configuration window of the PCI bus, made on the first ask and
/// kept: the root task enumerates through it and `app-lspci` is given a
/// handle to the same object.
fn window(gate: &mut Gate, world: &mut World) -> Option<(MemoryHandle, BusRange)> {
    if world.ecam.is_none() {
        world.ecam = ecam(gate, world).ok().flatten();
    }
    world.ecam
}

/// The virtio block device, found and prepared on the first ask and kept.
///
/// It is not done at the start of the machine, where every other object of
/// the root task is made, because the console is not up there and a refusal
/// would be a boot that says nothing.
fn block_device<'a>(gate: &mut Gate, world: &'a mut World) -> Option<&'a Block> {
    if world.block.is_none() {
        match find_block(gate, world) {
            Ok(found) => {
                // What the handover left behind, read back off the
                // device: the risk this reduces is a message table that
                // took the write and kept nothing, which no later step
                // would name.
                if let Some(block) = &found {
                    let line: Line<160> = Line::of(format_args!(
                        "[init] block msix bar{}+{:#x} of {} vectors holds {:#x}/{:#x} masked={}\n",
                        block.table.table.bar,
                        block.table.table.offset,
                        block.table.vectors,
                        block.back.address,
                        block.back.data,
                        block.back.masked
                    ));
                    say(gate, world, line.as_bytes());
                }
                world.block = found;
            }
            Err(refused) => {
                let (first, frames) = refused.about.unwrap_or((0, 0));
                let line: Line<160> = Line::of(format_args!(
                    "[init] no block device: {} at {} ({first:#x}+{frames:#x})\n",
                    refused.error.message(),
                    refused.step
                ));
                say(gate, world, line.as_bytes());
            }
        }
    }
    world.block.as_ref()
}

/// The virtio block device of the machine, prepared for a driver, or
/// nothing when the machine carries none.
///
/// The window is mapped one bus at a time, as `app-lspci` maps it and for
/// the same reason: one bus is one mebibyte of page tables (8.15).
fn find_block(gate: &mut Gate, world: &mut World) -> Result<Option<Block>, Refused> {
    let system = world.system.ok_or(Error::AccessDenied).step("system")?;
    let own = world.own;
    let Some((memory, buses)) = window(gate, world) else {
        return Ok(None);
    };
    for bus in buses.first_bus..=buses.last_bus {
        let offset = u64::from(bus.saturating_sub(buses.first_bus)).saturating_mul(BYTES_PER_BUS);
        let mut mapping =
            Mapping::window(gate, own, memory, BUS, offset, BYTES_PER_BUS).step("bus")?;
        let outcome = on_one_bus(gate, system, own, &mut mapping, buses, bus);
        mapping.unmap(gate, own).step("bus back")?;
        if let Some(block) = outcome? {
            return Ok(Some(block));
        }
    }
    Ok(None)
}

/// The device on this bus, prepared, or nothing when it is on another.
fn on_one_bus(
    gate: &mut Gate,
    system: SystemControlHandle,
    own: ProcessHandle,
    mapping: &mut Mapping,
    buses: BusRange,
    bus: u8,
) -> Result<Option<Block>, Refused> {
    let window = Window::new(buses.segment, bus, bus)
        .map_err(pci_error)
        .step("window")?;
    // SAFETY: the mapping stands until it is unmapped by the caller, it is
    // one mebibyte of device memory of this process alone, and the value
    // built here is the only one that reaches those bytes.
    let bytes = unsafe { mapping.bytes() };
    let mut space = MappedSpace::new(window, Mmio::of(bytes));
    let mut found = None;
    enumerate(&space, window, |function| {
        if found.is_none()
            && function.header.vendor == VIRTIO_VENDOR
            && function.header.device == BLOCK_DEVICE
        {
            found = Some(function.address);
        }
    })
    .map_err(pci_error)
    .step("enumerate")?;
    match found {
        Some(address) => prepare(gate, system, own, &mut space, address).map(Some),
        None => Ok(None),
    }
}

/// Makes the objects a driver of `address` is given, and leaves the device
/// able to raise its message interrupt.
///
/// The order is the order the specification asks for: the registers are
/// probed while the function decodes nothing, the table entry is written
/// while the vector is masked, and the function is enabled last.
fn prepare(
    gate: &mut Gate,
    system: SystemControlHandle,
    own: ProcessHandle,
    space: &mut MappedSpace<'_>,
    address: Address,
) -> Result<Block, Refused> {
    let header = pci::header::read(space, address)
        .map_err(pci_error)
        .step("header")?
        .ok_or(Error::NotFound)
        .step("header")?;
    let capabilities = walk(space, address, &header)
        .map_err(pci_error)
        .step("capabilities")?;
    let structures = virtio::structures(space, address, &capabilities)
        .map_err(pci_error)
        .step("structures")?;
    let msix_capability = find(&capabilities, ID_MSIX)
        .ok_or(Error::Unsupported)
        .step("msix capability")?;
    let table = msix::read(space, address, msix_capability.offset)
        .map_err(pci_error)
        .step("msix")?;
    if table.vectors == 0 {
        return Err(Error::Unsupported).step("vectors");
    }
    let bars = probe(space, address).map_err(pci_error).step("probe")?;
    let (index, mut places, multiplier) = layout(&structures).step("layout")?;
    let register = bar(&bars, index).step("register")?;
    // The object covers whole frames, so a register that does not start at
    // one leaves bytes below it inside the window. The driver counts from
    // the window, which is what it is given; the shift is added once here
    // rather than told to every driver as a fourth number.
    let shift = window_offset(register);
    for place in &mut places {
        place.offset = place.offset.saturating_add(shift);
    }
    let registers = device_memory(gate, system, register)?;
    // The decode bits go on before anything is written into a register of
    // the device. `probe` leaves the command register as the firmware left
    // it, and a firmware enables the decode of what it uses itself; the
    // entry of the message table is written through a register, so a write
    // before this reaches nothing at all. `COMMAND_BUS_MASTER` is what
    // lets the device read the region the driver hands it and write the
    // message that says it is done.
    let command = read_command(space, address)
        .map_err(pci_error)
        .step("command")?;
    write_command(
        space,
        address,
        command | COMMAND_MEMORY | COMMAND_BUS_MASTER,
    )
    .map_err(pci_error)
    .step("decode")?;

    let vector = gate.interrupt_create_msi(system).step("vector")?;
    let notification = gate.notification_create().step("notification")?;
    gate.interrupt_bind(vector.interrupt, notification, BLOCK_VECTOR_BIT)
        .step("bind")?;
    let back = write_vector(gate, system, own, &bars, &table, &vector)?;

    msix::set_function_mask(space, address, &table, false)
        .map_err(pci_error)
        .step("unmask")?;
    msix::set_enabled(space, address, &table, true)
        .map_err(pci_error)
        .step("enable")?;

    let table = msix::read(space, address, msix_capability.offset)
        .map_err(pci_error)
        .step("msix back")?;
    Ok(Block {
        registers,
        places,
        multiplier,
        interrupt: vector.interrupt,
        notification,
        table,
        back,
    })
}

/// Which base address register the four structures lie in, where each of
/// them lies in the window over that register, and what the notification
/// structure scales a queue index by.
///
/// The four have to share one register: the driver is given one window,
/// and a device that spread them over two is one this system does not
/// drive.
fn layout(
    structures: &[Option<virtio::Structure>; virtio::MAX_STRUCTURES],
) -> Result<(u8, [Location; 4], u32), Error> {
    let mut places = [Location { offset: 0, len: 0 }; 4];
    let mut register: Option<u8> = None;
    let mut multiplier = 0u32;
    for (slot, kind) in places.iter_mut().zip(STRUCTURES) {
        let found = structures
            .iter()
            .flatten()
            .find(|structure| structure.kind == kind)
            .ok_or(Error::Unsupported)?;
        if *register.get_or_insert(found.bar) != found.bar {
            return Err(Error::Unsupported);
        }
        *slot = Location {
            offset: found.offset,
            len: found.len,
        };
        if let Some(scale) = found.multiplier {
            multiplier = scale;
        }
    }
    let index = register.ok_or(Error::Unsupported)?;
    Ok((index, places, multiplier))
}

/// The memory register at `index`, or a refusal for one that decodes
/// nothing or decodes ports.
fn bar(bars: &[Option<Bar>; pci::bar::MAX_BARS], index: u8) -> Result<Bar, Error> {
    let found = bars
        .get(usize::from(index))
        .copied()
        .flatten()
        .ok_or(Error::Unsupported)?;
    match found.space {
        BarSpace::Memory { .. } => Ok(found),
        BarSpace::Io => Err(Error::Unsupported),
    }
}

/// A device memory object over the whole of `register`, aligned outward to
/// frames.
fn device_memory(
    gate: &mut Gate,
    system: SystemControlHandle,
    register: Bar,
) -> Result<MemoryHandle, Refused> {
    let (first, frames) = frames_of(register);
    gate.memory_create_device(system, first, frames)
        .map_err(|error| Refused {
            step: "window",
            error,
            about: Some((first, frames)),
        })
}

/// The first frame of `register` and how many frames cover it.
fn frames_of(register: Bar) -> (u64, u64) {
    let start = register.base & !PAGE_SIZE.wrapping_sub(1);
    let end = register
        .base
        .wrapping_add(register.len)
        .next_multiple_of(PAGE_SIZE);
    let bytes = end.saturating_sub(start);
    (
        start.wrapping_div(PAGE_SIZE),
        bytes.wrapping_div(PAGE_SIZE).max(1),
    )
}

/// How far into the window over `register` the register itself begins.
fn window_offset(register: Bar) -> u32 {
    let start = register.base & !PAGE_SIZE.wrapping_sub(1);
    u32::try_from(register.base.saturating_sub(start)).unwrap_or(0)
}

/// Writes the vector the kernel allocated into the entry of the MSI-X
/// table, through a window the root task maps and takes back again.
///
/// The table is in a register of the device and not in its configuration
/// space, so the bytes have to be reached through a mapping. It is often
/// not the register the four structures lie in, which is why the object is
/// made here and closed again rather than taken from the driver's window.
fn write_vector(
    gate: &mut Gate,
    system: SystemControlHandle,
    own: ProcessHandle,
    bars: &[Option<Bar>; pci::bar::MAX_BARS],
    table: &msix::MsiX,
    vector: &MessageInterrupt,
) -> Result<msix::Entry, Refused> {
    let register = bar(bars, table.table.bar).step("msix register")?;
    let memory = device_memory(gate, system, register)?;
    let (_, frames) = frames_of(register);
    let bytes = frames.saturating_mul(PAGE_SIZE);
    let outcome = write_entry_into(gate, own, memory, bytes, register, table, vector);
    let closed = gate.handle_close(memory.handle());
    let back = outcome.step("entry")?;
    closed.step("entry back")?;
    Ok(back)
}

/// Maps that window, writes the entry, and takes the window back.
///
/// The sixteen bytes are computed into a buffer and then stored through
/// [`Mmio`], one word at a time. That is what `pci::msix` asks of a
/// caller: the table lies in a register of the device, and a register is
/// read and written volatile or not at all — a store through a plain
/// reference to it may be dropped, and the entry then reads back as the
/// zeros the device reset it to.
///
/// The vector control word goes last, because it is what unmasks the
/// vector, and the address and the data have to stand before the device
/// may use them (*PCI Express Base Specification* 6.0, 7.7.2.3).
fn write_entry_into(
    gate: &mut Gate,
    own: ProcessHandle,
    memory: MemoryHandle,
    bytes: u64,
    register: Bar,
    table: &msix::MsiX,
    vector: &MessageInterrupt,
) -> Result<msix::Entry, Error> {
    let mut mapping = Mapping::new(gate, own, memory, MSIX, bytes)?;
    let entry = msix::Entry {
        address: vector.address,
        data: vector.data,
        masked: false,
    };
    let mut words = [0u8; msix::ENTRY_LEN];
    let built = msix::write_entry(&mut words, &entry).map_err(pci_error);
    let at = usize::try_from(
        u64::from(window_offset(register))
            .wrapping_add(u64::from(table.table.offset))
            .wrapping_add(
                u64::from(BLOCK_MSIX_ENTRY)
                    .wrapping_mul(u64::try_from(msix::ENTRY_LEN).unwrap_or(0)),
            ),
    )
    .unwrap_or(usize::MAX);
    // SAFETY: the window is mapped, it is device memory of this process
    // alone, and nothing else holds a reference to those bytes.
    let slot = unsafe { mapping.bytes() };
    let mut window = Mmio::of(slot);
    let stored = built.and_then(|()| store_entry(&mut window, at, &words));
    let back = read_back(&window, at);
    let unmapped = mapping.unmap(gate, own);
    stored?;
    unmapped?;
    Ok(back)
}

/// Stores the sixteen bytes of an entry at `at`, the control word last.
fn store_entry(
    window: &mut Mmio<'_>,
    at: usize,
    words: &[u8; msix::ENTRY_LEN],
) -> Result<(), Error> {
    for index in [0usize, 1, 2, 3] {
        let from = index.saturating_mul(4);
        let word = words
            .get(from..from.saturating_add(4))
            .and_then(|bytes| bytes.first_chunk::<4>().copied())
            .map_or(0, u32::from_le_bytes);
        if !window.write_u32(at.saturating_add(from), word) {
            return Err(Error::InvalidArgument);
        }
    }
    Ok(())
}

/// The entry the table holds at `at`, read back through the same window.
fn read_back(window: &Mmio<'_>, at: usize) -> msix::Entry {
    let mut words = [0u8; msix::ENTRY_LEN];
    for index in [0usize, 1, 2, 3] {
        let from = index.saturating_mul(4);
        let word = window.read_u32(at.saturating_add(from)).unwrap_or(0);
        if let Some(slot) = words.get_mut(from..from.saturating_add(4)) {
            slot.copy_from_slice(&word.to_le_bytes());
        }
    }
    msix::read_entry(&words).unwrap_or(msix::Entry {
        address: 0,
        data: 0,
        masked: true,
    })
}

/// Where one bus of the configuration window is mapped while it is
/// enumerated, and where the MSI-X table is mapped while its entry is
/// written.
///
/// Neither is [`SCRATCH`]: the enumeration runs while a child is being
/// given what it starts with, and the page that becomes that child's IPC
/// buffer is mapped at `SCRATCH` for as long as the startup message is
/// written into it. The two also differ from each other, because the entry
/// is written while the bus the device was found on is still mapped.
const BUS: u64 = 0x0000_5000_0000_0000;
const MSIX: u64 = 0x0000_5100_0000_0000;

/// The error a refusal of the `pci` crate becomes. The bus is what the
/// firmware left; a machine this program cannot read the bus of carries no
/// device it can hand over, which is `Unsupported` and no fault of a
/// caller.
const fn pci_error(_error: PciError) -> Error {
    Error::Unsupported
}

/// The ports of the PS/2 controller and the two lines its devices assert.
///
/// The range is one range and not two because `Role::IoPorts` is given once,
/// so `0x61`, `0x62`, and `0x63` fall inside it. Of those the kernel reaches
/// only `0x61`, the gate of the interval timer, and only while it calibrates
/// that timer at bring-up, which is over before this program runs.
fn ps2(
    gate: &mut Gate,
    world: &World,
) -> Result<(IoPortHandle, InterruptHandle, InterruptHandle), Error> {
    let system = world.system.ok_or(Error::AccessDenied)?;
    let ports = gate.ioport_create(system, PS2_PORT, PS2_PORTS)?;
    let keyboard = gate.interrupt_create(system, PS2_KEYBOARD_LINE)?;
    let mouse = gate.interrupt_create(system, PS2_MOUSE_LINE)?;
    Ok((ports, keyboard, mouse))
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
    if let Some(memory) = world.memory_for_self {
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
/// programs after it can be given one — and so that the root task itself
/// has one to ask, badged, because a request without a badge names nobody.
fn remember(
    gate: &mut Gate,
    world: &mut World,
    program: &Program,
    endpoint: EndpointHandle,
) -> Result<(), Error> {
    match program.name {
        b"server-memory" => {
            world.memory = Some(endpoint);
            world.memory_for_self = Some(gate.endpoint_badge(endpoint, INIT_BADGE)?);
        }
        b"server-name" => world.names = Some(endpoint),
        b"server-console" => {
            world.console = Some(endpoint);
            world.console_for_self = Some(gate.endpoint_badge(endpoint, INIT_BADGE)?);
        }
        b"server-display" => world.display = Some(endpoint),
        b"server-input" => world.input = Some(endpoint),
        _ => {}
    }
    Ok(())
}

/// Says what came of starting a program.
fn report(gate: &mut Gate, world: &World, name: &[u8], outcome: Result<(), Failure>) {
    match outcome {
        Ok(()) => {
            let mut line: Line<96> = Line::new();
            line.put(b"[init] started ");
            line.put(name);
            line.put(b"\n");
            say(gate, world, line.as_bytes());
        }
        Err(failure) => {
            let mut line: Line<192> = Line::new();
            line.put(b"[init] ");
            line.put(name);
            line.put(b" at ");
            line.put(failure.step.as_bytes());
            line.put(b": ");
            line.put(failure.error.message().as_bytes());
            line.put(b"\n");
            say(gate, world, line.as_bytes());
        }
    }
}

/// Puts a line out: through the console once there is one, and through the
/// kernel's own before that.
fn say(gate: &mut Gate, world: &World, line: &[u8]) {
    match world.console_for_self {
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
fn serve_faults(gate: &mut Gate, world: &World, mut outstanding: usize) -> ! {
    loop {
        let Ok(received) = gate.ipc_recv(world.faults) else {
            gate.thread_exit()
        };
        // One endpoint carries two kinds of news: a fault, which the kernel
        // sends under a label of its own, and a child's report that it is
        // done. The label says which.
        if let Ok(parent::Request::Finished { status }) = parent::Request::decode(gate.reader()) {
            let mut line: Line<96> = Line::new();
            line.put(b"[init] child ");
            put_number(&mut line, received.badge);
            line.put(b" finished with ");
            put_number(&mut line, status);
            line.put(b"\n");
            say(gate, world, line.as_bytes());
            outstanding = outstanding.saturating_sub(1);
            if outstanding == 0 {
                end_machine(gate, world)
            }
            continue;
        }
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

/// Ends the machine, which is the last thing the root task does.
///
/// The kernel cannot do it any more: once the root task runs, the kernel
/// only answers calls. So the root task takes the port of the exit device
/// through its `SystemControl` and writes to it, and the end of the run
/// goes through the userland like everything before it.
///
/// On a machine with no such device the write reaches nothing and the
/// thread exits, which is the right outcome: a system whose work is done
/// and that has no way to switch the machine off stops.
fn end_machine(gate: &mut Gate, world: &World) -> ! {
    say(gate, world, b"[init] all children are done\n");
    if let Some(system) = world.system
        && let Ok(ports) = gate.ioport_create(system, EXIT_PORT, EXIT_PORTS)
    {
        let _written = gate.ioport_write(ports, EXIT_PORT, EXIT_PORTS, EXIT_SUCCESS);
    }
    gate.thread_exit()
}

/// Writes `value` into `line` in decimal.
fn put_number<const N: usize>(line: &mut Line<N>, value: u64) {
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
