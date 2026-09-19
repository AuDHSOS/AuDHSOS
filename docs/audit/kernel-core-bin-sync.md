# kernel-core, audhsos-kernel, audhsos-sync audit findings

Repository: AuDHSOS/AuDHSOS. Audit of kernel-core, audhsos-kernel, audhsos-sync at main bc47df5.
Each `## ` section is one issue, filed 2026-09-19 with the number in its heading. `Title:` is the issue title, `Labels:` the labels, everything after `Body:` is the issue body verbatim.

---

## F01 — issue #113
Title: audhsos-kernel: a user fault of a vector without a kind reaches the handler as a general protection fault at address 0
Labels: bug, part::kernel
Body:
`crates/kernel/bin/src/main.rs:411-416` replaces a `None` from `Exception::fault` with a `Fault` of kind `GeneralProtection`, address `exception.cr2`, and delivers it through `task::deliver_fault`. `crates/kernel/core/src/trap.rs:95-99` states that a vector with no kind stops the thread without a message. `docs/02-architecture.md:256-258` names the six kinds a fault message carries. `crates/kernel/hal-x86_64/src/traps.rs:191` sets `fault_address` to `0` for every vector but 14, so the delivered address is `0`.

A user thread that sets the trap flag (`pushfq; or qword [rsp], 0x100; popfq`) raises vector 1 from user mode on the next instruction. `Exception::kind` at `crates/kernel/core/src/trap.rs:118-128` answers `None` for vector 1, and the fault handler of the process receives a message that names a general protection fault at address `0`. The same holds for vectors 4, 5, 7, 16 and 19, which a user thread raises with an unmasked x87 or SSE exception.

Fix: on `exception.fault() == None`, stop the thread through `kernel_syscall::fault::stop` and switch, writing no message, as `trap.rs:98-99` states; adding the missing vectors to `FaultKind` is the option not taken, because it changes the ABI.

---

## F02 — issue #114
Title: audhsos-kernel: the line of an unreported fault is written to no console
Labels: bug, part::kernel
Body:
`crates/kernel/bin/src/task.rs:440-456` builds the environment of `deliver_fault` with `console: None` at `task.rs:449`. `crates/kernel/syscall/src/fault.rs:92-97` states that a fault nobody handles writes a line through `machine.environment.log` at `fault.rs:111`, so that the machine does not go quiet without saying why. `crates/kernel/core/src/syscall.rs:276-280` drops the bytes when `console` is `None`.

A thread of the root task faults. The root task has no fault handler (`fault.rs:94-95`), so `unreported` runs and its line is dropped. The console shows the `[trap]` lines of `crates/kernel/bin/src/main.rs:400-403` and nothing that says the fault reached nobody. `answer` at `task.rs:287-301` takes the console for the same environment, so a system call logs and a fault does not.

Fix: build the environment of `deliver_fault` inside `entry::with_console` with `Some(console)`, in the order console, memory, machine that `task.rs:287-289` and D6 of `docs/16-more-than-one-processor.md` use.

---

## F03 — issue #116
Title: audhsos-sync: the crate documentation names a kernel token that does not exist
Labels: bug, part::kernel
Body:
`crates/sync/src/lib.rs:39-41` states that the kernel implements `ExclusiveToken` for its interrupt guard. No crate implements `ExclusiveToken` besides `UncontendedToken` at `crates/sync/src/lib.rs:48`. Every kernel borrow passes `UncontendedToken`: `crates/kernel/core/src/state.rs:68`, `crates/kernel/core/src/machine.rs:50`, `crates/kernel/core/src/memory.rs:855`, `crates/kernel/hal-x86_64/src/traps.rs:114` and `traps.rs:138`. `docs/16-more-than-one-processor.md:86` lists the kernel's token as missing until step S6.

A reader of the trait takes the token as a proof the kernel supplies. The proof today is the interrupt gate and the `sti` placement in `crates/kernel/bin/src/main.rs:82-114`, which the trait does not name.

Fix: state at `lib.rs:39-41` that every borrower passes `UncontendedToken` and that the kernel's token arrives with S6 of document 16.

---

## F04 — issue #118
Title: kernel-core: the trap counter of the kernel state stays zero
Labels: bug, part::kernel
Body:
`crates/kernel/core/src/state.rs:16-18` documents `KernelState::traps` as the number of traps the kernel has reported, and `crates/kernel/core/src/trap.rs:131-132` and `trap.rs:144-146` document `on_exception` and `on_user_fault` as counting into `state`. `crates/kernel/bin/src/main.rs:390` and `main.rs:400` pass a fresh `KernelState::new()` to both, and the value is dropped at the end of `on_trap`. The cell `KERNEL` at `state.rs:62` counts interrupts through `main.rs:210-220` and traps through nothing.

A user thread faults once per millisecond for a second. `KERNEL.traps` is `0` afterwards, and `KERNEL.interrupts` is about `1000`.

Fix: count the user fault through `kernel_core::with_state` at `main.rs:400`, which succeeds because a trap from user mode arrives while no cell is held, and keep the local state for the `StopMachine` path alone, where the cell may be held.

---

## F05 — issue #120
Title: kernel-core: the counters of the kernel state have no reader
Labels: enhancement, part::kernel
Body:
`crates/kernel/core/src/state.rs:16-26` holds four counters. `crates/kernel/bin/src/main.rs:210-220` writes three of them on every interrupt through `with_state`, which is one compare-and-exchange and one release store per interrupt at `crates/sync/src/lib.rs:114-123`. No code outside `crates/kernel/core/src/tests` reads `traps`, `ticks`, `interrupts` or `spurious`. `main.rs:220` binds the result to `let _ = counted;`, which does nothing.

The timer path of `on_interrupt` pays the borrow at 1000 Hz for a count that `kernel_hal_x86_64::interrupts::ticks()` at `main.rs:242` already answers.

Fix: delete `KernelState`, the `KERNEL` cell and the `with_state` call in `on_interrupt`; reporting the counters through `system_info` is the option not taken, because the tick count is already the clock.

---

## F06 — issue #122
Title: audhsos-kernel: the IPC buffer frame of the idle thread is the kernel page-table root
Labels: enhancement, part::kernel
Body:
`crates/kernel/bin/src/task.rs:161` reads `memory.root()`, and `task.rs:176` passes that frame to `Thread::new` as `ipc_buffer`. Three paths write or free the `ipc_buffer` of a thread by id: `announce` at `crates/kernel/bin/src/main.rs:318-343`, `write_result` at `crates/kernel/syscall/src/calls/ipc.rs:509-518`, and the reaper at `crates/kernel/syscall/src/reaper.rs:105`.

No handle names the idle thread, the idle thread waits on nothing, and the reaper spares the running thread, so no path reaches the frame today. A path that does writes a status word into the active PML4 or frees the root frame into the reserve.

Fix: give the idle thread a zeroed frame of the reserve through `Environment::allocate_frame`, as `root::build` does at `crates/kernel/core/src/root.rs:134`.

---

## F07 — issue #124
Title: kernel-core: a kernel region records the permissions of its first page for pages that have other permissions
Labels: enhancement, part::kernel
Body:
`crates/kernel/core/src/memory.rs:592-603` takes `perms` from the first mapped page of a run and inserts one `Region` for the whole run. `run_length` at `memory.rs:610-622` ends a run at an unmapped page and not at a change of permissions. `crates/kernel/bin/kernel.ld:12-28` places `.text`, `.rodata`, `.data` and `.bss` page-aligned and contiguous.

The bring-up registers one `Image` region for the whole kernel image with the permissions of `.text`, read and execute, over the pages of `.data` and `.bss`, which are mapped read and write. The report at `memory.rs:879-887` prints the backing and not the permissions, and nothing else reads `Region.perms` of the kernel table, so the wrong value has no reader today.

Fix: end the run in `run_length` when `mapper.translate` answers permissions that differ from those of the first page.

---

## F08 — issue #127
Title: audhsos-kernel: the frame range of the boot image undercounts by one frame for an unaligned start
Labels: enhancement, part::kernel
Body:
`crates/kernel/bin/src/task.rs:433-434` builds the range from `PhysFrame::containing(start)` and `len.div_ceil(PAGE_SIZE)`. `frames_of` at `crates/kernel/core/src/memory.rs:804-812` rounds both ends outward for the same purpose.

For `start = 0x1800` and `len = 0x1000` the bytes span the frames at `0x1000` and `0x2000`, and `task.rs:434` counts one frame. The root task then receives a `BootImage` memory object that ends one frame before the archive does. The loader places the image with `AllocatePages`, so the start is aligned today.

Fix: make `frames_of` public in `kernel_core::memory` and use it in `grants`.

---

## F09 — issue #129
Title: audhsos-kernel: translate copies the frame allocator onto the stack
Labels: enhancement, part::kernel
Body:
`crates/kernel/bin/src/task.rs:145` copies `*memory.frames()` into a local to build a `Mapper` for one `translate` call at `task.rs:152`. `BitmapFrameAllocator` at `crates/kernel/mm/src/frame_allocator.rs:74-79` holds `[u64; 256]` at `frame_allocator.rs:23`, so the copy is 2 KiB plus three words on the boot stack. `translate` allocates nothing.

`KernelMemory::frames_mut` at `crates/kernel/core/src/memory.rs:164-166` gives the `&mut` the `Mapper` constructor asks for.

Fix: pass `memory.frames_mut()` to `Mapper::new` in `translate`.

---

## F10 — issue #131
Title: audhsos-kernel: comments place the scheduler in a future phase that has arrived
Labels: enhancement, part::kernel
Body:
`crates/kernel/core/src/tick.rs:19-21` states that a tick gives the scheduler its turn and that until there are threads to switch between the turn is the count. `tick.rs:27-28` states that Phase 5 puts the scheduler into `tick::schedule`. The scheduler runs in `on_timer_tick` at `crates/kernel/bin/src/main.rs:241-266`, and `tick::schedule` at `tick.rs:29-31` returns the count. `main.rs:177-179` states that Phase 5 replaces the wait for three ticks with a scheduler; the wait stands at `main.rs:174` and `main.rs:182-189` beside the scheduler.

A reader of `tick.rs` looks for the scheduler in the wrong crate.

Fix: state at `tick.rs:19-31` that `on_tick` counts and that `main.rs:241-266` schedules, and state at `main.rs:177-179` that the wait proves the timer runs before the root task starts.

---

## F11 — issue #134
Title: kernel-core: the environment constructor is documented with six borrows and takes seven parameters
Labels: enhancement, part::kernel
Body:
`crates/kernel/core/src/syscall.rs:103` names six, and `syscall.rs:130-131` states that the six the constructor takes are borrows of the machine. `KernelEnvironment::new` at `syscall.rs:104-112` takes seven parameters, and `acpi` and `prepare` are values.

Fix: state seven parameters, of which five are borrows.
