// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

//! Starting the root task and running what it starts.
//!
//! This is where the kernel stops being the only thing that runs. It builds
//! the one process nobody could have built for it, hands it everything, and
//! then becomes the idle thread: from here on the kernel only answers
//! system calls and interrupts.
//!
//! The parts that know nothing of a processor are in
//! [`kernel_core::root`]; what is left here is the frame a new thread
//! returns through, the switch of the stacks, and the task state segment
//! that says where a trap from user mode lands.

use crate::runtime::{with_machine, with_memory};
use crate::switching::{executing, record_executing, saved_context};
use audhsos_abi::WallClockSource;
use audhsos_abi::layout::{
    BOOT_STACK_TOP, KERNEL_STACK_PAGES, KERNEL_STACK_SLOT_PAGES, KERNEL_STACKS_BASE, PAGE_SIZE,
};
use core::sync::atomic::{AtomicI64, AtomicU32, AtomicU64, Ordering};
use kernel_core::memory::KernelMemory;
use kernel_core::root::{self, Grants, RootTask};
use kernel_core::syscall::{KernelEnvironment, handle_syscall, reap, schedule, store_context};
use kernel_core::{memory, println};
use kernel_hal_x86_64::bootinfo::X86Platform;
use kernel_hal_x86_64::console::SerialConsole;
use kernel_hal_x86_64::paging::{AddressSpaces, X86Entry};
use kernel_hal_x86_64::ports::DeviceAccess;
use kernel_hal_x86_64::remote::SharedTlb;
use kernel_hal_x86_64::window::PhysicalWindow;

use kernel_hal_x86_64::processor::KernelToken;
use kernel_hal_x86_64::{context, descriptors, entry};

use kernel_types::{PhysFrame, PhysFrameRange, VirtAddr};

/// The kernel stack slot the idle thread uses, which is the boot stack this
/// image already stands on.
const BOOT_SLOT: u32 = u32::MAX;

/// Builds the root task out of the boot image and starts it.
///
/// Returns `false` when there is no root task to start, which is a boot
/// image the loader should have refused.
pub(crate) fn start(platform: &X86Platform) -> bool {
    use kernel_hal_api::platform::Platform as _;
    ACPI.store(
        platform
            .acpi_rsdp()
            .map_or(0, kernel_types::PhysAddr::as_u64),
        Ordering::Relaxed,
    );
    if let Some((seconds, source)) = platform.wall_clock() {
        WALL_SECONDS.store(seconds, Ordering::Relaxed);
        WALL_SOURCE.store(source.code(), Ordering::Relaxed);
    }
    let Some(program) = root_task_bytes(platform) else {
        return false;
    };
    let Some((image_frames, ram)) = grants(platform) else {
        return false;
    };
    let mut regions = [PhysFrameRange::EMPTY; audhsos_abi::layout::MAX_BOOT_REGIONS];
    let mut count = 0usize;
    for (slot, region) in regions.iter_mut().zip(ram.iter()) {
        *slot = *region;
        count = count.wrapping_add(1);
    }

    let built = with_memory(|memory| {
        with_machine(|machine| {
            let mut tables = window();
            let mut tlb = SharedTlb::new();
            let mut environment = environment(memory, &mut tables, &mut tlb, None);
            let grants = Grants {
                boot_image: image_frames,
                ram: regions.get(..count).unwrap_or(&[]),
            };
            root::build(&mut environment, &mut machine.objects, program, &grants)
        })
    })
    .flatten();
    let task = match built {
        Some(Ok(task)) => task,
        Some(Err(error)) => {
            entry::with_console(|console| println!(console, "[root] {error}"));
            return false;
        }
        None => return false,
    };

    if !prepare(&task) {
        return false;
    }
    idle_thread(BOOT_SLOT);
    with_machine(|machine| {
        let _started = machine
            .scheduler
            .start(&mut machine.objects.threads, task.thread);
    });
    entry::with_console(|console| {
        println!(
            console,
            "root task at {:#x}, stack {:#x}, buffer {:#x}",
            task.entry.as_u64(),
            task.stack.as_u64(),
            task.buffer.as_u64()
        );
    });
    true
}

/// Writes the frame the first switch into the root task returns through.
fn prepare(task: &RootTask) -> bool {
    let Some(below) = task.kernel_stack_top.checked_sub(PAGE_SIZE) else {
        return false;
    };
    let Ok(page) = kernel_types::Page::from_start(below) else {
        return false;
    };
    let Some(top_frame) = translate(page) else {
        return false;
    };
    let mut stack_window = window();
    let Some(context) = context::prepare_user(
        &mut stack_window,
        top_frame,
        task.kernel_stack_top,
        task.entry,
        task.stack,
        task.buffer,
    ) else {
        return false;
    };
    with_machine(|machine| store_context(&mut machine.objects, task.thread, context)).is_some()
}

/// The frame `page` of the kernel half is mapped to.
fn translate(page: kernel_types::Page) -> Option<PhysFrame> {
    let mut tables = window();
    let mut tlb = SharedTlb::new();
    with_memory(|memory| {
        let root = memory.root();
        let mapper = kernel_mm::mapper::Mapper::<'_, X86Entry, _, _, _>::new(
            root,
            &mut tables,
            &mut tlb,
            memory.frames_mut(),
        );
        mapper.translate(page).map(|(frame, _)| frame)
    })
    .flatten()
}

/// The kernel's own process and the idle thread, which is this image: the
/// thread the kernel switches back into when nothing else can run. Its
/// context is written by the first switch away from it.
pub(crate) fn idle_thread(slot: u32) {
    let mut tables = window();
    let mut tlb = SharedTlb::new();
    with_memory(|memory| {
        let root = memory.root();
        with_machine(|machine| {
            if machine.scheduler.idle().is_some() {
                return;
            }
            let mut environment = environment(memory, &mut tables, &mut tlb, None);
            let Ok(idle) = root::idle(&mut environment, &mut machine.objects, root, slot) else {
                return;
            };
            if let Ok(thread) = machine.objects.threads.get_mut(idle) {
                thread.cpu = machine.scheduler.caller();
            }
            machine.scheduler.set_idle(idle);
            // The kernel is that thread: it is what the first switch leaves.
            machine.scheduler.adopt(idle);
            record_executing(idle);
        });
    });
}

/// Refuses a same-processor nested borrow before switching stacks.
fn nothing_is_held() -> bool {
    !memory::MEMORY.is_held_by(&KernelToken)
        && !kernel_core::machine::MACHINE.is_held_by(&KernelToken)
        && !entry::console_is_held()
}

/// Runs whichever thread the scheduler picks, and comes back when the
/// processor is standing on this stack again.
///
/// A call made while the thread that would leave holds a cell of the
/// kernel returns at once and switches nobody — see [`nothing_is_held`].
///
/// `standing_on` names the thread whose kernel stack the processor stands
/// on after the scheduler has let go of it, which is what a thread that
/// faulted looks like.
pub(crate) fn run(standing_on: Option<kernel_objects::object::ThreadId>) {
    if !nothing_is_held() {
        return;
    }
    let Some((from, to, top)) = next_switch(standing_on) else {
        sweep();
        return;
    };
    // A thread that has ended has nowhere to keep a context any more, and
    // nobody will ask for it: the switch writes it here, on the stack that
    // goes with it.
    let mut discarded = VirtAddr::ZERO;
    let from = from.unwrap_or(&raw mut discarded);
    if descriptors::set_kernel_stack(top.as_u64()).is_err() {
        return;
    }
    // SAFETY: the fixed-home context slot is independent of the machine cell;
    // only this processor accesses it, and both stacks remain mapped.
    unsafe {
        context::switch_to(from, to);
    }
    // Back on this stack, a turn of the processor later: whatever ended
    // while we were away can go.
    sweep();
}

/// Gives back what every thread that has ended held.
pub(crate) fn sweep() {
    if with_machine(|machine| machine.scheduler.ended()) == Some(0) {
        return;
    }
    let mut tables = window();
    let mut tlb = SharedTlb::new();
    with_memory(|memory| {
        with_machine(|machine| {
            let mut environment = environment(memory, &mut tables, &mut tlb, None);
            let running = executing();
            let _cleared = reap(
                &mut machine.objects,
                &mut machine.scheduler,
                &mut environment,
                running,
            );
        });
    });
}

/// Answers the system call of `caller` and clears away what ended.
///
/// The devices are held for the length of the call, which turns interrupts
/// off while it runs: the calls that touch a line or a port need that, and
/// no other call minds.
pub(crate) fn answer(
    caller: kernel_objects::object::ThreadId,
    devices: Option<&mut DeviceAccess<'_>>,
) -> bool {
    let mut tables = window();
    let mut tlb = SharedTlb::new();
    let mut devices = devices;
    // The console goes in: `debug_log` is what the root task says anything
    // through until it has handed the serial port to the console driver,
    // and after that the console gives itself up and writes nothing.
    entry::with_console(|console| {
        with_memory(|memory| {
            with_machine(|machine| {
                let Ok(thread) = machine.objects.threads.get(caller) else {
                    return true;
                };
                if thread.state != audhsos_abi::ThreadState::Running {
                    return true;
                }
                let mut buffer_window = window();
                let Some(bytes) = buffer_window.frame_bytes_mut(thread.ipc_buffer) else {
                    return true;
                };
                let mut environment =
                    KernelEnvironment::<X86Entry, _, _, _, DeviceAccess<'_>>::new(
                        memory,
                        &mut tables,
                        &mut tlb,
                        Some(console),
                        devices.take(),
                        acpi_pointer(),
                        context::prepare_user,
                    )
                    .at(now_micros())
                    .with_wall_clock(wall_clock());
                let reschedule = handle_syscall(
                    &mut machine.objects,
                    &mut machine.scheduler,
                    &mut environment,
                    caller,
                    bytes,
                );
                let _cleared = reap(
                    &mut machine.objects,
                    &mut machine.scheduler,
                    &mut environment,
                    Some(caller),
                );
                reschedule
            })
        })
        .flatten()
    })
    .flatten()
    .unwrap_or(false)
}

/// Delivers the fault of `faulted` to whoever handles it, and answers
/// whether the caller should switch before it returns.
///
/// `None` is a vector with no kind: the thread stops and no handler hears
/// of it. A stopped thread is off the processor, so the caller passes it to
/// [`run`] as the thread it is standing on.
///
/// The console goes in, in the order of [`answer`], so the line of a fault
/// nobody handles is written.
pub(crate) fn deliver_fault(
    faulted: kernel_objects::object::ThreadId,
    fault: Option<audhsos_abi::Fault>,
) -> bool {
    let mut tables = window();
    let mut tlb = SharedTlb::new();
    entry::with_console(|console| {
        with_memory(|memory| {
            with_machine(|machine| {
                let Ok(thread) = machine.objects.threads.get(faulted) else {
                    return true;
                };
                if thread.state != audhsos_abi::ThreadState::Running {
                    return true;
                }
                let mut buffer_window = window();
                let Some(bytes) = buffer_window.frame_bytes_mut(thread.ipc_buffer) else {
                    return true;
                };
                let mut environment = environment(memory, &mut tables, &mut tlb, Some(console));
                let mut syscall = kernel_syscall::dispatch::Machine {
                    objects: &mut machine.objects,
                    scheduler: &mut machine.scheduler,
                    environment: &mut environment,
                };
                kernel_syscall::fault::take(&mut syscall, faulted, fault, bytes).reschedule
            })
        })
        .flatten()
    })
    .flatten()
    .unwrap_or(false)
}

/// The next switch, or `None` when the thread that should run is the one
/// that is running. The first element is where the context of the thread
/// that leaves belongs, and `None` when it has ended.
fn next_switch(
    standing_on: Option<kernel_objects::object::ThreadId>,
) -> Option<(Option<*mut VirtAddr>, VirtAddr, VirtAddr)> {
    let mut spaces = AddressSpaces::current().ok()?;
    with_machine(|machine| {
        let next = schedule(
            &mut machine.objects,
            &mut machine.scheduler,
            &mut spaces,
            stack_top_of,
        );
        let switch = next.switch?;
        let leaving = executing().or(switch.from).or(standing_on);
        if leaving == Some(switch.to) {
            return None;
        }
        let pointer = leaving.and_then(|from| {
            let initial = machine.objects.threads.get(from).ok()?.context;
            Some(saved_context(from, initial)?.as_ptr().cast::<VirtAddr>())
        });
        let context =
            VirtAddr::new(saved_context(switch.to, switch.context)?.load(Ordering::Relaxed))
                .ok()?;
        record_executing(switch.to);
        Some((pointer, context, switch.kernel_stack_top))
    })
    .flatten()
}

/// The top of the kernel stack in `slot`: the slot begins with a guard page
/// and holds [`KERNEL_STACK_PAGES`] mapped pages above it. The idle thread
/// is the exception: it is this image, standing on the boot stack.
fn stack_top_of(slot: u32) -> VirtAddr {
    if slot == BOOT_SLOT {
        return VirtAddr::new(BOOT_STACK_TOP).unwrap_or(VirtAddr::ZERO);
    }
    let base = KERNEL_STACKS_BASE
        .saturating_add(u64::from(slot).saturating_mul(KERNEL_STACK_SLOT_PAGES * PAGE_SIZE));
    let top = base
        .saturating_add(PAGE_SIZE)
        .saturating_add(KERNEL_STACK_PAGES.saturating_mul(PAGE_SIZE));
    VirtAddr::new(top).unwrap_or(VirtAddr::ZERO)
}

/// The bytes of the root task inside the boot image.
fn root_task_bytes(platform: &X86Platform) -> Option<&'static [u8]> {
    use kernel_hal_api::platform::Platform as _;
    let (start, len) = memory::boot_image(platform)?;
    // SAFETY: the physical window maps every frame of memory the loader
    // reported, and the boot image is one of its regions; the bytes are
    // never written after the loader placed them.
    let image = unsafe { kernel_hal_x86_64::memory::physical_slice(start, len) }?;
    let header = audhsos_abi::BootImageHeader::parse(
        image,
        len,
        kernel_core::boot::usable_bytes(platform.memory_regions()),
    )
    .ok()?;
    let from = usize::try_from(header.root_task_offset).ok()?;
    let to = from.checked_add(usize::try_from(header.root_task_len).ok()?)?;
    image.get(from..to)
}

/// The frames of the boot image and the free regions of memory.
fn grants(platform: &X86Platform) -> Option<(PhysFrameRange, heapless::Regions)> {
    let (start, len) = memory::boot_image(platform)?;
    let frames = memory::frames_of(start.as_u64(), len)?;
    let free = with_memory(|memory| heapless::Regions::of(memory.free()))?;
    Some((frames, free))
}

/// The environment the calls of this image reach the machine through.
fn environment<'a>(
    memory: &'a mut KernelMemory,
    tables: &'a mut PhysicalWindow,
    tlb: &'a mut SharedTlb,
    console: Option<&'a mut SerialConsole>,
) -> KernelEnvironment<'a, X86Entry, PhysicalWindow, SharedTlb, SerialConsole, DeviceAccess<'a>> {
    KernelEnvironment::new(
        memory,
        tables,
        tlb,
        console,
        None,
        acpi_pointer(),
        context::prepare_user,
    )
    .at(now_micros())
    .with_wall_clock(wall_clock())
}

/// The clock the system call layer answers with: the ticks the timer has
/// counted, in microseconds. The count is an atomic of the interrupt
/// module, so reading it borrows nothing.
fn now_micros() -> u64 {
    kernel_core::tick::micros(kernel_hal_x86_64::interrupts::ticks())
}

/// The physical address of the root system description pointer, which is
/// the only thing of the firmware the kernel keeps. It is read once, while
/// the platform is still in hand, and kept here because the paths that
/// answer a system call are not given one.
static ACPI: AtomicU64 = AtomicU64::new(0);

/// The pointer the bring-up read.
fn acpi_pointer() -> u64 {
    ACPI.load(Ordering::Relaxed)
}

/// The moment the firmware clock stood at when the loader read it, in
/// seconds from the Unix epoch, and the code of the source it came from.
/// Both are kept here for the reason [`ACPI`] is: the paths that answer a
/// system call are handed no platform.
///
/// A source of zero is a machine that reported no clock, which is what a
/// fresh pair says, so a kernel whose boot information carried none needs
/// no further mark.
static WALL_SECONDS: AtomicI64 = AtomicI64::new(0);

/// The code of the [`WallClockSource`] the loader reported; `0` for none.
static WALL_SOURCE: AtomicU32 = AtomicU32::new(0);

/// The wall clock the loader read, as the system call layer takes it.
fn wall_clock() -> Option<(i64, WallClockSource)> {
    let source = WallClockSource::from_code(WALL_SOURCE.load(Ordering::Relaxed))?;
    Some((WALL_SECONDS.load(Ordering::Relaxed), source))
}

/// The physical window, which maps every frame of memory.
const fn window() -> PhysicalWindow {
    // SAFETY: the kernel tables are active, so the window maps every
    // physical frame read and write, and the kernel is the only writer.
    unsafe { PhysicalWindow::kernel() }
}

/// A fixed list of memory regions, so that the free map can be carried out
/// of the borrow that produced it.
pub(crate) mod heapless {
    use audhsos_abi::layout::MAX_BOOT_REGIONS;
    use kernel_types::PhysFrameRange;

    /// Up to [`MAX_BOOT_REGIONS`] regions.
    #[derive(Clone, Copy, Debug)]
    pub(crate) struct Regions {
        items: [PhysFrameRange; MAX_BOOT_REGIONS],
        len: usize,
    }

    impl Regions {
        /// The regions `iter` yields, up to what the list holds.
        pub(crate) fn of(map: &kernel_mm::memory_map::NormalizedMap) -> Self {
            let mut regions = Regions {
                items: [PhysFrameRange::EMPTY; MAX_BOOT_REGIONS],
                len: 0,
            };
            for (slot, range) in regions.items.iter_mut().zip(map.iter()) {
                *slot = range;
                regions.len = regions.len.wrapping_add(1);
            }
            if let Some(used) = regions.items.get_mut(..regions.len) {
                used.sort_unstable_by_key(|range| core::cmp::Reverse(range.count()));
            }
            regions
        }

        /// The regions, in order.
        pub(crate) fn iter(&self) -> impl Iterator<Item = &PhysFrameRange> {
            self.items.iter().take(self.len)
        }
    }
}
