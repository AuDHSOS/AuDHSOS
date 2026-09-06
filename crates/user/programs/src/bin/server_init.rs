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
use user_proto::parent;

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
    /// Whether it reports to this program when it is done. The machine
    /// ends when every program that reports has reported.
    reports: bool,
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
        reports: true,
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
        console_for_self: None,
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
    /// The same, badged for the root task's own lines.
    console_for_self: Option<EndpointHandle>,
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
    world: &World,
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
    let mut given: [(Role, Handle); 8] = [(Role::OwnProcess, Handle::MAX); 8];
    let mut count = 0usize;

    let own = gate.process_install_handle(child, child.handle(), ObjectRights::PROCESS)?;
    push(&mut given, &mut count, Role::OwnProcess, own);
    let served = gate.process_install_handle(child, endpoint.handle(), ObjectRights::RECV)?;
    push(&mut given, &mut count, Role::OwnEndpoint, served);

    // What a child holds of a server is a capability with the child's badge
    // on it: the badge is the only thing about a sender a server can trust,
    // and a request that arrives without one names nobody.
    if program.names
        && let Some(names) = world.names
    {
        let marked = gate.endpoint_badge(names, badge)?;
        let handle = gate.process_install_handle(child, marked.handle(), ObjectRights::SEND)?;
        push(&mut given, &mut count, Role::NameServer, handle);
    }
    if program.memory
        && let Some(memory) = world.memory
    {
        let marked = gate.endpoint_badge(memory, badge)?;
        let handle = gate.process_install_handle(child, marked.handle(), ObjectRights::SEND)?;
        push(&mut given, &mut count, Role::MemoryServer, handle);
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
        push(&mut given, &mut count, Role::Parent, handle);
    }
    if let Some(console) = world.console
        && program.grant != Grant::Serial
    {
        let marked = gate.endpoint_badge(console, badge)?;
        let handle = gate.process_install_handle(child, marked.handle(), ObjectRights::SEND)?;
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
