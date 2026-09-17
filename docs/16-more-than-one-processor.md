# 16. More than one processor

## 16.0 How to read this document

Every step below has the same six parts, in the same order: Status,
Depends on, Size, Needs, Does, Produces, Done when. Nothing is implied.
Every term is defined in 16.1. Every term has exactly one name, used
everywhere.

## 16.1 Terms

| Term | Meaning |
|------|---------|
| processor | One logical processor of the machine. The reference machine has as many as `-smp` gives it. |
| boot processor | The processor the firmware started. The boot processor runs `kernel_entry` today. |
| application processor | Every processor that is not the boot processor. |
| processor number | An index `0..CPUS`, given in the order of the processor list. Processor number 0 is the boot processor. |
| local APIC identifier | The number the hardware gives a processor's local APIC, readable at register `0x20`. A local APIC identifier is not a processor number. |
| processor list | The processors the firmware reports, read out of the `MADT`. |
| processor table | The array of per-processor state, indexed by the processor number. |
| start-up page | The 4 KiB page below 1 MiB holding the code an application processor runs between reset and its first Rust call. |
| start-up code | The instructions in the start-up page. |
| parameter block | The part of the start-up page the kernel writes before it starts a processor: page-table root, entry address, stack top, processor number, descriptor table. |
| interprocessor interrupt | An interrupt one processor sends to another through the interrupt command register. Abbreviated IPI. |
| `INIT-SIPI-SIPI` | The three IPIs that bring an application processor from reset to the start-up page, *Intel SDM* Vol. 3A, 11.4.4.1. |
| interrupt command register | Local APIC registers `0x300` and `0x310`, which send an IPI, *Intel SDM* Vol. 3A, 13.6.1. Abbreviated ICR. |
| kernel cell | One of the four `static` cells the kernel borrows: the console, the memory, the machine, the controller. |
| machine cell | `MACHINE` in `crates/kernel/core/src/machine.rs`, line 44, holding the object pools and the run queues. |
| home processor | The processor whose run queue a thread is enqueued in. Fixed when the thread is created. |
| remote invalidation | Removing a translation from the translation lookaside buffer of a processor other than the one that changed the page table. |
| *Intel SDM* | *Intel 64 and IA-32 Architectures Software Developer's Manual*, order number 325462-092US, June 2026. Step S1 puts it in `docs/intel/`. |

## 16.2 Goal

At the end of the track, five things are true that are not true now:

1. The kernel runs user threads on more than one processor at the same
   time.
2. Each processor has its own run queue, its own current thread, and its
   own idle thread.
3. One processor may take the machine cell while another waits for it,
   and neither loses work.
4. A processor that changes a page table removes the stale translation
   from every processor that has the address space loaded.
5. A machine started with `-smp 1` behaves exactly as it does today.

## 16.3 What is already built

Nine pieces of the SMP path exist and need no change.

| Piece | What it gives | Where |
|-------|---------------|-------|
| `Scheduler` | Run queues, current thread, idle thread, time slice. The doc comment names one processor. | `crates/kernel/sched/src/scheduler.rs`, line 74 |
| `TlbControl` | A trait with `flush_page` and `flush_all`, which `kernel-mm` calls and never implements. | `crates/kernel/hal-api/src/paging.rs`, line 51 |
| I/O APIC routing | A redirection entry already carries the local APIC identifier it delivers to. | `crates/kernel/x86-tables/src/ioapic.rs`, line 87 |
| Message interrupts | An MSI address already carries the local APIC identifier it delivers to. | `crates/kernel/hal-x86_64/src/apic.rs`, line 50 |
| `StackPool` | Allocates and releases one kernel stack of eight pages plus a guard page. | `crates/kernel/mm/src/stack.rs`, line 230 |
| `read_msr`, `write_msr` | Model-specific register access, already within the `asm!` budget. | `crates/kernel/hal-x86_64/src/instructions.rs`, line 348 |
| `invalidate_page` | `invlpg` on the processor that calls it. | `crates/kernel/hal-x86_64/src/instructions.rs`, line 258 |
| The coarse critical section | Eighteen call sites outside the tests take the whole machine cell at once, which is the shape a big kernel lock needs. | `crates/kernel/core/src/machine.rs`, line 49 |
| The userland runtime | Holds no `static`. Each thread receives its own IPC buffer at entry. | `crates/user/rt/src`, `crates/user/sys-x86_64/src/lib.rs`, line 152 |

D-17 fixes per-processor run queues, a big kernel lock first, and
per-object locks only after measurement; 2.5.3 of
[document 2](02-architecture.md) repeats that decision. This document
implements it rather than reopening it.

## 16.4 What is missing

| # | What is missing | Where it has to go | Step |
|---|-----------------|--------------------|------|
| 1 | The manual that every constant of this track cites. | `docs/intel/` | S1 |
| 2 | A machine with more than one processor. The runner pins `-smp 1`. | `crates/tools/xtask/src/qemu.rs`, line 462 | S2 |
| 3 | A processor list. The `MADT` parser counts local APIC entries and keeps none of them, and reads no x2APIC entry. | `crates/kernel/acpi/src/madt.rs`, line 287 | S3 |
| 4 | The interrupt command register. The local APIC module has no `0x300`, no `0x310`, and no IPI encoding. | `crates/kernel/x86-tables/src/lapic.rs` | S4 |
| 5 | A borrow that waits. `acquire` is one compare-and-exchange that answers `AlreadyBorrowed` and returns; a second processor would read that as "the cell is not there" and skip the work. | `crates/sync/src/lib.rs`, lines 114 and 249 | S5 |
| 6 | Per-processor descriptor tables. One `TSS_IMAGE`, one `GDT`, one double-fault stack, all `static`. | `crates/kernel/hal-x86_64/src/descriptors.rs`, lines 29 to 38 | S6 |
| 7 | An end-of-interrupt that takes no shared cell. `acknowledge` goes through the controller cell, which a system call holds for its whole length. | `crates/kernel/hal-x86_64/src/interrupts.rs`, line 154; `crates/kernel/bin/src/main.rs`, line 146 | S6 |
| 8 | The start-up code of an application processor, and the frame below 1 MiB it is copied into. The bring-up removes the loader's identity mapping. | `crates/kernel/core/src/memory.rs`, line 646 | S7 |
| 9 | A second idle thread. The idle thread stands on the boot stack, and the machine has one boot stack. | `crates/kernel/bin/src/task.rs`, line 40 | S8 |
| 10 | Run queues per processor. `Machine` holds one `Scheduler`, and eighteen signatures take `&mut Scheduler`. | `crates/kernel/core/src/machine.rs`, line 23; `crates/kernel/syscall/src/dispatch.rs`, line 38 | S9 |
| 11 | Remote invalidation. `LocalTlb` invalidates on the calling processor and nowhere else. | `crates/kernel/hal-x86_64/src/paging.rs`, line 24 | S10 |
| 12 | A clock that counts once. `TICKS` is one counter and every processor's timer would raise it. | `crates/kernel/hal-x86_64/src/timer.rs`, line 81 | S6 |

## 16.5 Decision D1: how a processor finds its own data

This decision must be made before S6 starts. It decides what every trap
handler does first.

**The decision: a processor reads its local APIC identifier from register
`0x20` through a window base held in an atomic, and looks the processor
number up in `IDENTIFIERS`, sixteen atomics the boot processor writes
once.**

Neither the base nor the table is a kernel cell. `IDENTIFIERS` is
`[AtomicU32; CPUS]`, `u32::MAX` for a number no processor has, and
`APIC_WINDOW` is an `AtomicU64` holding the address
`interrupts::bring_up` maps (`crates/kernel/hal-x86_64/src/interrupts.rs`,
line 105). The boot processor writes both before it starts any
application processor.

Reason 1: `processor()` may borrow no cell, because the token of D5 calls
it to answer `owner` and the borrow is what that token decides. A borrow
inside the call that says who holds the borrow does not terminate.
Reason 2: the processor list the identifiers come from lives in `Apics`
inside the controller cell (`crates/kernel/hal-x86_64/src/interrupts.rs`,
line 79), which D6 orders first, so reading the list where it lies is
that same cycle. `IDENTIFIERS` is the copy outside every cell.
Reason 3: the register window of the local APIC is mapped once and every
processor reading that address reaches its own unit, *Intel SDM* Vol. 3A,
13.4.4. One mapping serves every processor.
Reason 4: the search is a linear scan over `CPUS` entries, which is a
constant of sixteen, so the lookup is O(1) and needs no allocation.
Result: `processor()` answers a processor number from anywhere, including
a trap handler and the inside of a borrow, and costs one uncached read
and one scan. It reads the register through a raw pointer and not through
`LocalApic::id` (`crates/kernel/hal-x86_64/src/apic.rs`, line 141),
because reaching a `LocalApic` value means borrowing a cell; that read is
the seventeenth `unsafe` site D8 names. The cost lands on every borrow of
a kernel cell, because the token of D5 carries the number; measurement 1
of S11 is what reads it.

**The option not taken: `GS_BASE` and `swapgs`.** A per-processor pointer
in `IA32_KERNEL_GS_BASE`, swapped at every entry from user mode, is one
register read instead of one uncached MMIO read. It costs a `swapgs` in
every trap entry path and a second one in every exit path, which is a
change to code generated by the `x86-interrupt` ABI rather than to code
this project writes. 8.18 of [document 8](08-roadmap.md) is where it
belongs, after S11 measures whether the read is worth removing.

## 16.6 Decision D2: what produces the start-up code

This decision must be made before S7 starts. R3 of
[document 4](04-safety-policy.md) forbids `global_asm!`, assembly files,
and an assembler invoked from a build script
(`crates/tools/xtask/src/unsafe_budget.rs`, line 344, refuses
`global_asm!` outright), and an application processor starts in real mode
at a page below 1 MiB, which is neither where the linker puts code nor the
mode it compiles for.

### Why the start-up code is not Rust

A processor at reset has no stack, no paging, sixteen-bit registers and no
sixty-four-bit mode. `rustc` emits sixty-four-bit code only. These are the
instructions between reset and the first Rust call, and what stops each
from being Rust:

| Instruction | Why Rust cannot write it |
|-------------|--------------------------|
| Read the processor's own base out of `CS` | Sixteen-bit code; `rustc` emits none. |
| `lgdt` | The instruction has no Rust form. |
| Set `CR4.PAE`, load `CR3`, set `IA32_EFER.LME`, set `CR0.PG` and `CR0.PE` | Control registers; the same. |
| Far-jump into the sixty-four-bit segment | Rust has no far jump. |
| Load the stack pointer | Rust needs a stack before it runs, so no Rust code can be the one that sets it. |
| Call the kernel | Nothing: this is the first Rust instruction, and every instruction after it is Rust. |

About twenty instructions, run once per processor at start-up and never
again. No assembler program enters the build: `naked_asm!` is the
compiler's, and `kernel-hal-x86_64` holds thirty of its sites today
(`crates/tools/xtask/src/policy.rs`, line 692). D8 makes this the
thirty-first.

**The decision: one `#[unsafe(naked)]` function whose body is
`naked_asm!`, placed in its own output section, copied into the start-up
page at run time.**

Reason 1: `naked_asm!` counts against the `asm!` budget, which D8 raises
by a named site; `global_asm!` is refused whatever the budget.
Reason 2: the code is written position-independently — it takes its own
base from `CS`, which after a start-up IPI holds the vector shifted left
by eight (*Intel SDM* Vol. 3A, 11.4.4.1, step 10) — so the kernel may copy
it to whichever low frame is free rather than to one fixed address.
Reason 3: the linker gives `__apstart_start` and `__apstart_end` for the
section, so the copy has a length and needs no marker instruction to find
its own end.
Result: `kernel.ld` gains a section, `kernel-hal-x86_64` gains one
`naked_asm!` site, and the kernel copies `__apstart_end - __apstart_start`
bytes.

**The option not taken: a `const [u8; N]` of hand-encoded instructions.**
It needs no `asm!` at all and is a pure function the host can test, which
fits the way this project builds descriptor tables and register words. It
costs about ninety bytes of opcodes encoded by hand against Vol. 2 of the
*Intel SDM*, and a host test of those bytes proves the encoder and not the
sequence: nothing on the host can run sixteen-bit code. A defect in it
appears as a processor that never reports, with no way to read back what
it executed.

## 16.7 Decision D3: which processor keeps the clock

This decision must be made before S6 starts. Every processor has a local
APIC timer, and the kernel has one tick counter.

**The decision: the boot processor's timer raises `TICKS`; an application
processor's timer charges the time slice of its own current thread and
raises nothing.**

Reason 1: `kernel_core::tick::micros` converts `TICKS` into microseconds
since boot, and a counter four processors raise runs four times too fast.
Reason 2: a deadline is a time and not a share of a processor, so one
processor expiring deadlines is enough; `kernel_ipc::expire` already walks
the deadline list to the first entry that has not passed, O(woken).
Reason 3: a time slice is a share of a processor, so each processor has to
charge its own.
Result: `on_timer_tick` splits in two. The boot processor runs what it
runs today. An application processor charges its own current thread and
switches when the slice is spent.

**The option not taken: one processor sends a tick IPI to the others.** It
keeps one timer and makes the slice exact across processors. It costs one
IPI per processor per millisecond, which on the reference machine under
TCG is the dominant cost of an idle machine.

## 16.8 Decision D4: how a thread reaches a run queue

This decision must be made before S9 starts. A wake happens on whichever
processor holds the machine cell, and the woken thread has to be enqueued
somewhere.

**The decision: a thread is given a home processor when it is created, in
round robin over the processors that are up, and it is enqueued there for
its whole life.**

Reason 1: every scheduler operation stays what it is — `Scheduler` is
unchanged and its host tests are unchanged.
Reason 2: a wake needs one addition and one modulo to find the queue,
O(1), and no scan of the other processors.
Reason 3: a thread that never changes queue needs no lock of its own on
the queue, because the machine cell already covers every queue.
Result: `Thread` gains `cpu: u8`, `Machine` holds `CPUS` schedulers, and a
wake that enqueues on a processor other than the caller's sends that
processor a reschedule IPI.

**The option not taken: one shared run queue.** It balances perfectly and
needs no home processor. It contradicts D-17, and it makes the ready
bitmap of the one queue the most written word in the kernel.

**The option not taken: work stealing.** An idle processor takes a thread
from the longest queue. It balances what round robin does not — a machine
whose threads have unequal appetites. It costs a second owner for a
thread's queue links, which is exactly the per-object locking D-17 defers
until S11 has measured.

## 16.9 Decision D5: what the borrow does to the processor that holds it

This decision must be made before S5 starts. Today a borrow of a kernel
cell that finds the cell taken answers `AlreadyBorrowed`, and every caller
reads that as "not now, and that is correct": a timer tick inside a system
call changes nothing and the next tick tries again (D-133).

**The decision: the cell records which processor holds it. The processor
that holds it is answered `AlreadyBorrowed`, as today. Another processor
waits.**

Reason 1: every existing caller keeps its meaning. A tick that interrupts
a system call on the same processor still changes nothing, which is what
2.5.4 of [document 2](02-architecture.md) and D-133 describe.
Reason 2: a second processor that were answered `AlreadyBorrowed` would
drop the work rather than defer it — a system call that answers nothing,
not a tick that skips.
Reason 3: the owner is supplied by the token the borrow already takes, so
no call site changes. `ExclusiveToken` gains `fn owner(&self) -> u32`
with a default of `0`, `UncontendedToken` keeps the default, and every
userland caller therefore behaves exactly as today because it can never
see a second owner.
Result: `borrowed: AtomicBool` becomes `owner: AtomicU32` with
`u32::MAX` for free. The wait is a compare-and-exchange loop that calls
`ExclusiveToken::wait`, whose default is `core::hint::spin_loop` — a safe
function that emits `pause` and costs no `asm!` site.

**The option not taken: a re-entrant lock.** The owner would be admitted
and a depth counter kept. It costs the guarantee that makes the
non-preemptible kernel safe: a handler admitted into a cell a system call
is halfway through reads a half-written machine.

## 16.10 Decision D6: the order of the locks

This decision must be made before S5 starts. Four kernel cells exist and a
wait turns two taken in the wrong order into a machine that stops.

**The decision: the order is controller, console, memory, machine. A
processor that holds one may take only a cell later in the list.**

The order is the one the code already has, and it is fixed rather than
chosen:

| Where | What it nests |
|-------|---------------|
| `crates/kernel/bin/src/main.rs`, line 146 | controller, then everything `task::answer` takes |
| `crates/kernel/bin/src/task.rs`, line 287 | console, then memory, then machine |
| `crates/kernel/bin/src/main.rs`, line 46 | console, then memory |
| `crates/kernel/bin/src/task.rs`, line 71 | memory, then machine |

`forward` takes the machine cell and gives it back before it takes the
controller (`crates/kernel/bin/src/main.rs`, lines 283 to 297), so those
two are held one after the other and not one inside the other.

**The one cycle a wait would create, and what removes it.** A processor
waiting for a cell waits with interrupts off, so it answers no IPI; a
processor that waits for every other to acknowledge a remote invalidation
while one of them waits for a cell it holds would wait forever. The wait
therefore polls: `ExclusiveToken::wait` of the kernel's token calls
`remote::poll`, which performs the invalidations asked of this processor
and acknowledges them, without an interrupt and without taking any cell.
S10 builds it.

**The option not taken: waiting with interrupts on.** It needs no polling.
It admits a timer tick into a processor that is inside a wait, and that
tick takes the same cell the wait is for, so the wait nests one level per
interrupt.

## 16.11 Decision D7: how many processors

**The decision: `CPUS` is sixteen. A machine that reports more is used up
to sixteen and reports how many it left.**

Reason 1: a `Queue` is two `Option<ThreadId>` of twelve bytes, and a
`Scheduler` is thirty-two of them plus six fields, about 830 bytes, so
sixteen schedulers are 13 KiB of `.bss`.
Reason 2: sixteen is four times the largest `-smp` the check needs, so a
raise is not what a wider acceptance run asks for first.
Reason 3: the processor number is a `u8` field of `Thread` and stays one.
Where: `kernel_core::config`, beside `THREADS` and `KERNEL_STACKS`.

**The option not taken: as many as the firmware reports.** A `Scheduler`
per processor would then be a run-time allocation out of the kernel
reserve, which is `unsafe` in a logic crate and is what D-57 refused for
the object pools.

## 16.12 Decision D8: the `unsafe` and `asm!` budgets

This decision must be made before S6 starts. Three crates stand exactly at
their budget today, so the first commit of S6 fails
`sh tools/xtask.sh unsafe-budget` before it does anything else.

| Crate | Now | After | Where the budget stands |
|-------|-----|-------|-------------------------|
| `audhsos-sync` | 4 unsafe, 0 asm | 4 unsafe, 0 asm | `crates/tools/xtask/src/policy.rs`, line 454 |
| `kernel-hal-x86_64` | 152 unsafe, 30 asm | 169 unsafe, 31 asm | `crates/tools/xtask/src/policy.rs`, line 691 |
| `audhsos-kernel` | 33 unsafe, 0 asm | 37 unsafe, 0 asm | `crates/tools/xtask/src/policy.rs`, line 712 |

`audhsos-sync` gains nothing: the wait replaces one compare-and-exchange
with a loop around it and reaches the value through the `slot` that is
already there.

The seventeen new `unsafe` sites of `kernel-hal-x86_64`:

| Sites | Step | What they do |
|-------|------|--------------|
| 3 | S4 | Write `0x310`, write `0x300`, read the delivery status bit. |
| 4 | S6 | Build a per-processor task state segment, load it, load the per-processor global descriptor table, load the interrupt descriptor table on an application processor. |
| 2 | S6 | Construct a second `LocalApic` over the window the boot processor mapped, and enable it. |
| 1 | S6 | Read the local APIC identifier register through `APIC_WINDOW`, which is `processor()` of D1. |
| 5 | S7 | Copy the start-up code into the page, write the parameter block, read the two section symbols, take the address of the Rust entry, enter the kernel from the start-up page. |
| 2 | S10 | Invalidate a page named by another processor; read the request word of this processor. |

The one new `asm!` site is the `naked_asm!` of D2, in S7.

The four new `unsafe` sites of `audhsos-kernel` are the entry an
application processor lands on, the switch into its idle thread, and the
two calls that turn interrupts on and off around them.

**The option not taken: a budget stated as a total rather than site by
site.** A named count is checkable: an eighteenth site in
`kernel-hal-x86_64` is a change to this decision and not to a number.

## 16.13 The order of the steps

| Step | Name | Status | Depends on | Size |
|------|------|--------|------------|------|
| S1 | The manual on the disk | not built | nothing | S |
| S2 | The machine carries more than one processor | not built | nothing | S |
| S3 | The processor list | not built | S1 | M |
| S4 | The interrupt command register | not built | S1 | M |
| S5 | The borrow that waits | not built | D5 (16.9), D6 (16.10) | M |
| S6 | Per-processor data | not built | S3, S5, D1 (16.5), D3 (16.7), D8 (16.12) | L |
| S7 | The start-up page and the first application processor | not built | S2, S4, S6, D2 (16.6) | XL |
| S8 | The application processor idles | not built | S5, S7 | L |
| S9 | Per-processor run queues | not built | S8, D4 (16.8), D7 (16.11) | XL |
| S10 | Remote invalidation | not built | S8 | L |
| S11 | Measurement, and what it decides | not built | S9, S10 | M |

S1 and S2 depend on nothing. S3 and S4 depend only on S1 and may be
built in either order. S6 depends on S5 because the token S6 installs at
the four cells is the one S5 builds. S9 and S10 are independent of each
other and may be built at the same time.

## 16.14 S1. The manual on the disk

Status: not built.
Depends on: nothing.
Size: S.

### Needs (already built)

- `docs/uefi/README.md` and `docs/ti/README.md`, which are the form: what
  is here, where it came from, why it is here.
- D-59, which records that a reference document is a document and not a
  dependency: `Cargo.lock` still lists only workspace members.
- D-124, which states the test a document has to pass to be kept beside
  the code: whether it can be obtained.

### Does

1. Create `docs/intel/`.
2. Fetch the manual and check it against the table below.
3. Write `docs/intel/README.md` in the form of `docs/uefi/README.md`.

| Field | Value |
|-------|-------|
| File | `docs/intel/325462-092-sdm-vol-1-2abcd-3abcd-4.pdf` |
| Document | *Intel 64 and IA-32 Architectures Software Developer's Manual*, combined volumes 1, 2A–2D, 3A–3D and 4, order number 325462-092US, June 2026 |
| Source | `https://cdrdv2.intel.com/v1/dl/getContent/671200` |
| Bytes | 26664910 |
| SHA-256 | `16a9336104750613ae2f2bab6eb7a1b21a7e1ef60ced35e9ab2e0d8c7efcec68` |

The redistribution question D-124 asks is answered in the manual's own
notices page: "you may publish an unmodified copy". The file is 26.6 MB
against a packed repository of 38 MB, which is the cost of the step and
the reason it is a step and not a line of another one.

### Produces

Every citation of this document from here on. The sections this track
reads are fixed:

| Section | Title | Used by |
|---------|-------|---------|
| Vol. 3A, 5.10.4 | Invalidation of TLBs and Paging-Structure Caches | S10 |
| Vol. 3A, 11.4 | Multiple-Processor (MP) Initialization | S7 |
| Vol. 3A, 11.4.4.1 | Typical BSP Initialization Sequence | S7 |
| Vol. 3A, 11.4.4.2 | Typical AP Initialization Sequence | S7 |
| Vol. 3A, 13.4.4 | Local APIC Status and Location | S6 |
| Vol. 3A, 13.4.6 | Local APIC ID | S6 |
| Vol. 3A, 13.6.1 | Interrupt Command Register (ICR) | S4 |
| Vol. 3A, 13.6.2.1 | Physical Destination Mode | S4 |

### Done when

1. `docs/intel/325462-092-sdm-vol-1-2abcd-3abcd-4.pdf` has the length and
   the digest of the table above.
2. `docs/intel/README.md` names the document, the retrieval, the digest,
   and what the code takes from it.
3. `docs/rfc/README.md` names `docs/intel/` beside `docs/oasis/`, the way
   it already does for OASIS.

### What is also settled here

`crates/kernel/x86-tables/src/lapic.rs` cites "the specification" and
names none. Every constant of that file gets the section it comes from,
which is what `docs/pcisig/README.md` calls the rule that takes the place
of a copy — except that here the copy is on the disk.

## 16.15 S2. The machine carries more than one processor

Status: not built.
Depends on: nothing.
Size: S.

### Needs (already built)

- `qemu::Options`, which already carries five per-run choices
  (`crates/tools/xtask/src/qemu.rs`, line 205).
- The probe machine that chooses KVM or TCG once per check.

### Does

1. `Options` gains `processors: u32` and loses `#[derive(Default)]` for a
   hand-written `Default` that sets `processors` to 1 and every other
   field to what the derive gave. The reason: the derive answers 0, and a
   machine started with `-smp 0` does not start.
2. The command line builder writes that number after `-smp`
   (`crates/tools/xtask/src/qemu.rs`, line 462).
3. 3.1.1 of [document 3](03-target-platform.md) states that `-smp` is a
   per-run choice and that 1 is the default.
4. The test of the command line builder
   (`crates/tools/xtask/src/tests/qemu.rs`, line 135) gains a case for a
   number other than 1.

### Done when

1. A run started with `processors: 4` reaches the same harness output as
   a run started with `processors: 1`, because nothing yet uses the other
   three.
2. `sh tools/xtask-check.sh` exits with 0.

## 16.16 S3. The processor list

Status: not built.
Depends on: S1.
Size: M.

### Needs (already built)

- The `MADT` walk, which already ends on a zero-length entry, refuses an
  entry that leaves the table, and refuses a table with more entries than
  the kernel holds (`crates/kernel/acpi/src/madt.rs`, lines 13 to 17).
- `MAX_IO_APICS` and `MAX_OVERRIDES`, which are the pattern a fixed-size
  list of processors follows.

### Does

1. Add `MAX_PROCESSORS`, equal to `CPUS` of D7.
2. Read entry type 0, the processor local APIC, whole: the ACPI processor
   identifier, the local APIC identifier, and the flags. *ACPI 6.6*,
   5.2.12.2, table 5.22.
3. Read entry type 9, the processor local x2APIC: the 32-bit local APIC
   identifier and the flags. *ACPI 6.6*, 5.2.12.12, table 5.34. A
   machine whose processor has an identifier above 254 reports that
   processor as a type 9 entry, so a parser that reads only type 0 loses
   it.
4. Keep a processor whose `Enabled` bit is set, as startable. Keep a
   processor whose `Enabled` is clear and whose `Online Capable` is set
   as present and not startable, which is the processor *ACPI 6.6*,
   table 5.23, says the firmware can enable at run time. Drop a
   processor with both bits clear, which the same table calls unusable.
5. Keep the order the table gives. The processor number is the position
   in the list.
6. Answer `TooManyProcessors` for a table with more than `MAX_PROCESSORS`
   usable entries, and not a truncated list — the same rule the I/O APICs
   already follow.

The walk stays O(entries) and the storage stays a fixed array.

### Produces

`Madt::processors` changes from a count to a list
(`crates/kernel/acpi/src/madt.rs`, line 164). It has three callers, all
of them tests: `crates/kernel/acpi/src/tests/madt.rs`, lines 40 and 63,
which read the count, and `crates/kernel/bin/tests/interrupts.rs`, line
209, which reads it in QEMU and asserts that the machine reports at least
one processor. The first two compare against a length, the third against
the length of the list.

### Done when

1. A host test parses a table with four type 0 entries and reads four
   identifiers in order.
2. A host test parses a table whose type 0 entry has `Enabled` clear and
   `Online Capable` clear, and keeps no processor from it.
3. A host test parses a table with a type 9 entry and reads a 32-bit
   identifier.
4. A host test parses a table with `MAX_PROCESSORS + 1` usable entries
   and reads `TooManyProcessors`.
5. A host test parses a table whose type 0 entry has `Enabled` clear and
   `Online Capable` set, and keeps a processor that is not startable.
6. The fuzz target of `kernel-acpi` runs against the new entry types.
7. 6.6.11 of [document 6](06-testing-strategy.md) lists the five cases.

## 16.17 S4. The interrupt command register

Status: not built.
Depends on: S1.
Size: M.

### Needs (already built)

- `kernel_x86_tables::lapic`, whose register offsets and local vector
  table encoding are built the same way.
- `kernel_x86_tables::vectors`, whose plan has 0x31 to 0x3F free between
  the timer and the first I/O APIC line.

### Does

1. Add `ICR_LOW = 0x300` and `ICR_HIGH = 0x310`.
2. Add the field encoding of *Intel SDM* Vol. 3A, figure 13-12: vector in
   bits 7:0, delivery mode in 10:8, destination mode in bit 11, delivery
   status in bit 12, level in bit 14, trigger mode in bit 15, destination
   shorthand in 19:18, destination in 63:56.
3. Add three constructors, each `const`, each setting the fields it names
   and leaving the reserved bits zero:

| Function | Delivery mode | Level | Trigger | Vector |
|----------|---------------|-------|---------|--------|
| `init(destination)` | 101 | assert | edge | 0 |
| `startup(destination, page)` | 110 | assert | edge | the page number |
| `fixed(destination, vector)` | 000 | assert | edge | the vector |

4. Add two vectors to the plan: `RESCHEDULE = 0x31` and
   `INVALIDATE = 0x32`, with the `const` assertions the file already
   makes for every other range.

### Produces

The encodings the *Intel SDM* gives as literals in 11.4.4.1 are then
computed: a broadcast INIT is `0x000C4500` and a broadcast start-up is
`0x000C46XX`. The host test checks `init` and `startup` with the
`All Excluding Self` shorthand against exactly those two words, which is
the one place this project can check its encoder against the manual's own
numbers.

### Done when

1. A host test reproduces `0x000C4500` and `0x000C46XX`.
2. A host test reads each field back out of each encoding.
3. A host test finds no overlap between the two new vectors and any range
   of the plan, which is a `const` assertion and therefore a compile
   error rather than a test failure.
4. 6.6.16 of [document 6](06-testing-strategy.md) lists the cases.

## 16.18 S5. The borrow that waits

Status: not built.
Depends on: D5 (16.9), D6 (16.10).
Size: M.

### Needs (already built)

- `Global` and `Preset`, whose `acquire` is one compare-and-exchange
  (`crates/sync/src/lib.rs`, lines 114 and 249) and whose `slot` is
  reached only by the holder of the flag (lines 132 and 266).
- R9 of [document 4](04-safety-policy.md), which runs `audhsos-sync`
  whole under Miri.

### Does

1. `ExclusiveToken` gains two provided methods: `fn owner(&self) -> u32`,
   default `0`, and `fn wait(&self)`, default `core::hint::spin_loop()`.
2. `Global::borrowed` and `Preset::borrowed` become
   `owner: AtomicU32`, free being `u32::MAX`.
3. `acquire` takes the owner and loops: compare-and-exchange free to
   owner; on failure, answer `AlreadyBorrowed` when the value read is
   this owner, and call `ExclusiveToken::wait` otherwise.
4. `release` stores free.
5. The four cells keep `UncontendedToken`. S6 installs the kernel's own
   token in their place, because that token's `owner` is `processor()`
   and D1 is built in S6; its `wait` is `remote::poll` of S10 followed by
   `spin_loop`, which is why S10 may be built after S6 but not left out.

`Global::init` keeps a plain `acquire`: a cell is written once, during
bring-up, by the boot processor.

### Produces

Nothing else changes. `borrow` keeps its signature, its error type, and
its meaning for every caller that passes `UncontendedToken`, whose owner
is always `0` and which therefore can never find a second owner.

### Done when

1. A host test borrows twice with one token and reads `AlreadyBorrowed`,
   which is the test that exists today and must still pass.
2. A Miri test with two threads and two tokens of different owners shows
   the second thread entering after the first releases, and shows no data
   race.
3. A Miri test with two threads and one owner shows the second borrow
   refused rather than waiting, which is the deadlock D5 forbids.
4. `sh tools/xtask.sh miri` covers the module, as R9 requires.
5. 6.6.18 of [document 6](06-testing-strategy.md) lists the three cases.

## 16.19 S6. Per-processor data

Status: not built.
Depends on: S3, S5, D1 (16.5), D3 (16.7), D8 (16.12).
Size: L.

### Needs (already built)

- `StackPool`, which hands out a kernel stack of eight pages with a guard
  page below it (`crates/kernel/mm/src/stack.rs`, line 230).
- `build_gdt(tss_base)`, which builds a whole table around one task state
  segment (`crates/kernel/x86-tables/src/gdt.rs`, line 121).
- `TaskStateSegment::with_kernel_stack` and `with_interrupt_stack`.
- The physical window, through which the kernel reaches any frame.

### Does

1. Add `Processor`, holding: the local APIC identifier, the global
   descriptor table, the task state segment image, the double-fault
   stack, a `LocalApic`, the tick count of this processor's timer, and
   the invalidation request word S10 fills.
2. Add the processor table, `PROCESSORS: [Preset<Processor>; CPUS]`, in
   `.bss` for the reason D-66 gives.
3. Add `APIC_WINDOW` and `IDENTIFIERS` of D1, and write both in
   `interrupts::bring_up` (`crates/kernel/hal-x86_64/src/interrupts.rs`,
   line 105) after the window is mapped and before any application
   processor starts. Neither is a cell, for the reason D1 gives.
4. Add `processor() -> Option<u8>`: read register `0x20` through
   `APIC_WINDOW`, scan `IDENTIFIERS`, answer the position. The scan is
   over `CPUS` entries and `CPUS` is a constant, so the call is O(1).
5. Install the kernel's token of S5 at the four cells, in place of
   `UncontendedToken`: its `owner` is `processor()`, its `wait` is
   `remote::poll` of S10 followed by `spin_loop`. Until S10 exists the
   `wait` is `spin_loop` alone, which one processor cannot deadlock on.
6. Split `descriptors::install`: what is one table stays one table, the
   interrupt descriptor table; what is per processor moves into
   `Processor`. The boot processor takes entry 0 and keeps the boot stack
   as its `RSP0`.
7. `set_kernel_stack` writes into the calling processor's own task state
   segment.
8. Give each `Processor` its own `LocalApic` over the window the boot
   processor mapped. `LocalApic::new` today requires that its value be
   the only one reaching the window; that requirement becomes: one value
   per processor, and a processor reaches only its own unit, which
   *Intel SDM* Vol. 3A, 13.4.4 is the ground for.
9. `acknowledge` uses the calling processor's `LocalApic` and takes no
   shared cell. This is the fix for gap 7 of 16.4: today a system call
   holds the controller cell for its whole length
   (`crates/kernel/bin/src/main.rs`, line 146), so an end-of-interrupt on
   another processor would wait for a system call to finish, and a local
   APIC that is not acknowledged delivers nothing after that.
10. Split `on_timer_tick` as D3 says.

### Produces

`processor()`, the call every later step makes to find its own data, and
an end-of-interrupt that costs one MMIO write and no wait.

### Done when

1. A kernel test in QEMU reads the processor number on the boot processor
   and gets 0, called both outside a borrow and inside one, which is the
   cycle D1 removes.
2. A kernel test reads `IDENTIFIERS[0]` and register `0x20` of the boot
   processor and finds the two equal.
3. A kernel test reads the boot processor's `RSP0` back out of its own
   task state segment and gets the boot stack top, which is what
   `descriptors::kernel_stack` already checks.
4. A kernel test raises a double fault and lands on the boot processor's
   interrupt-stack-table stack, which is the existing
   `double_fault.rs` test and must still pass.
5. A machine with `-smp 1` boots, runs the root task, and ends as it does
   today.
6. `sh tools/xtask.sh unsafe-budget` passes at the numbers D8 names.

## 16.20 S7. The start-up page and the first application processor

Status: not built.
Depends on: S2, S4, S6, D2 (16.6).
Size: XL.

### Needs (already built)

- `NormalizedMap::without`, which takes a range out of the usable memory
  (`crates/kernel/mm/src/memory_map.rs`, line 302). It is `pub(crate)` to
  `kernel-mm`, so the bring-up reaches it through `select_reserve`
  (`crates/kernel/mm/src/reserve.rs`, line 54) and S7 adds the entry
  point for the low frame beside it.
- The mapper, which maps any page of the kernel address space to any
  frame, and `drop_identity`, which shows how a low identity mapping is
  removed again (`crates/kernel/core/src/memory.rs`, line 646).
- `read_msr` and `write_msr`, for `IA32_EFER`.

### Does, in order

1. The memory bring-up takes the lowest usable frame at or above
   `0x1000` and below `0x100000` out of the map, the way it takes the
   kernel reserve. A machine with no such frame starts no application
   processor and says so.
2. Map that frame identity in the kernel address space, read and write.
   The address lies in the user half of the kernel root, which is the
   half the loader's own identity mapping used and which no user process
   shares.
3. Copy `__apstart_end - __apstart_start` bytes of the section into the
   frame.
4. Write the parameter block at the end of the page:

| Offset | Bytes | Contents |
|--------|-------|----------|
| `0xF00` | 4 | The page-table root the processor loads into `CR3`. |
| `0xF08` | 8 | The address of the Rust function the processor enters. |
| `0xF10` | 8 | The top of this processor's kernel stack. |
| `0xF18` | 8 | The processor number. |
| `0xF20` | 10 | The operand of `lgdt`: limit, then the linear address of the table below. |
| `0xF30` | 24 | Three descriptors: null, 32-bit code, 64-bit code. |
| `0xF48` | 6 | The operand of the far jump into the 64-bit segment: a 32-bit offset and a 16-bit selector. |

   The root at `0xF00` is four bytes because a real-mode
   `mov cr3, eax` carries thirty-two bits. The kernel refuses to start an
   application processor when its own root frame is at or above 4 GiB,
   which on a machine of 256 MiB it is not.

5. The start-up code does, in order: clear the interrupt flag; take its
   own base from `CS` and put it in `DS`; `lgdt`; set `CR4.PAE`; load
   `CR3`; set `IA32_EFER.LME`; set `CR0.PG` and `CR0.PE` together;
   far-jump through `0xF48` into the 64-bit part of the same page; load
   the stack top; put the processor number in the first argument
   register; jump to the address at `0xF08`.
6. Allocate a kernel stack for the application processor out of
   `StackPool` before it is started, because the start-up code cannot
   allocate.
7. Send `INIT`, wait 10 ms, send `STARTUP` with the page number, wait
   200 µs, send `STARTUP` again — *Intel SDM* Vol. 3A, 11.4.4.1, step 15,
   the right-hand column, because this kernel knows how many processors
   it expects.
8. Wait for the processor to raise its own entry of a started-flag array,
   with a deadline. A processor that does not report inside 100 ms is
   left alone and counted as not started.
9. Start one processor at a time. The reason is the calibration: it
   drives channel two of the interval timer
   (`crates/kernel/hal-x86_64/src/timer.rs`, line 26), which the machine
   has one of.
10. Unmap the identity page after the last processor has reported.

### Produces

`start_processors(list) -> usize`, which answers how many are running,
and the first Rust function an application processor reaches.

### Done when

1. A kernel test with `-smp 2` reports the local APIC identifier of the
   second processor, read by that processor itself.
2. A kernel test with `-smp 4` reports four identifiers, all different.
3. A kernel test with `-smp 1` reports one and starts nobody.
4. A kernel test finds the identity page unmapped after the bring-up, the
   way `memory.rs` already tests that the loader's identity mapping is
   gone.
5. A machine whose low memory holds no usable frame boots on the boot
   processor and says how many it left.

## 16.21 S8. The application processor idles

Status: not built.
Depends on: S5, S7.
Size: L.

### Needs (already built)

- `idle_thread`, which builds a thread on the boot stack and makes the
  scheduler adopt it (`crates/kernel/bin/src/task.rs`, line 160).
- `BOOT_SLOT`, the kernel stack slot number the idle thread uses
  (`crates/kernel/bin/src/task.rs`, line 40).
- The idle loop, and the reason it turns interrupts on before it halts
  (`crates/kernel/bin/src/main.rs`, line 82).

### Does

1. The application processor loads the interrupt descriptor table, its
   own global descriptor table and task state segment, and enables its
   local APIC.
2. It calibrates its own timer and starts it at `TICKS_PER_SECOND`.
3. It builds an idle thread of its own. `BOOT_SLOT` is the boot
   processor's; an application processor's idle thread takes a slot from
   `StackPool` — the stack S7 allocated for it, so that its idle thread
   and its start-up stack are the same stack.
4. It takes the machine cell, adopts its idle thread, gives the cell
   back.
5. It enters the idle loop.

### Produces

A machine on which two processors are inside the kernel at once, which
is what S9 and S10 are built on.

### Done when

1. A kernel test with `-smp 2` shows both processors taking and releasing
   the machine cell, counted by each and reported by the boot processor.
2. A kernel test shows a timer tick counted on each processor separately,
   and `TICKS` raised only by the boot processor, which is D3.
3. A kernel test shows an end-of-interrupt on the application processor
   while the boot processor holds the controller cell, and a second tick
   arriving after it — which is gap 7 of 16.4 and the reason S6 gave each
   processor its own `LocalApic`.
4. A machine with `-smp 1` behaves as it does today.

## 16.22 S9. Per-processor run queues

Status: not built.
Depends on: S8, D4 (16.8), D7 (16.11).
Size: XL.

### Needs (already built)

- `Scheduler`, unchanged, whose host tests stay as they are.
- `Outcome`, which says whether the caller should switch before it
  returns (`crates/kernel/sched/src/scheduler.rs`, line 61).
- The eighteen signatures that take `&mut Scheduler`, in `kernel-ipc`,
  `kernel-core` and `kernel-syscall`.

### Does

1. Add `Processors`, a logic type in `kernel-sched`: `[Scheduler; CPUS]`,
   the number of the calling processor, and a bitmask of the processors
   that need a reschedule IPI.
2. `Processors` offers the method surface `Scheduler` offers, and routes
   each call by the home processor of the thread it is given: `enqueue`,
   `dequeue`, `on_wake`, `on_block`, `suspend`, `resume`, `fault`,
   `exit`, `set_priority`, `start`, `expired`. Each is one index, O(1).
3. `current`, `idle`, `adopt`, `tick`, `pick_next`, `yield_now` act on
   the calling processor's own `Scheduler` and on no other.
4. A call that makes a thread ready on a processor other than the caller
   sets that processor's bit in the mask.
5. `Thread` gains `cpu: u8`. `thread_create` gives it in round robin over
   the processors that are up, which `kernel-objects` keeps as one
   counter.
6. Replace `scheduler: &mut Scheduler` with `scheduler: &mut Processors`
   at the eighteen sites and in `dispatch::Machine`
   (`crates/kernel/syscall/src/dispatch.rs`, line 38).
7. `Machine` holds `Processors` instead of `Scheduler`
   (`crates/kernel/core/src/machine.rs`, line 23).
8. The kernel binary reads the mask after it gives the machine cell back,
   clears it, and sends `RESCHEDULE` to each processor named. The mask is
   read after the cell is released, so no IPI is sent while the cell is
   held.
9. The handler of `RESCHEDULE` acknowledges at the local APIC and calls
   `task::run(None)`, which is what a timer tick that spends a slice
   already does.

### Produces

`Processors`, host-tested the way `Scheduler` is: a model test over four
schedulers where every operation names a thread, and the model says which
queue it lands in.

### Done when

1. A host test shows a thread woken by a processor other than its home
   processor landing in its home queue, and the bit of that processor
   set.
2. A host test shows a thread woken by its own processor setting no bit.
3. A kernel test with `-smp 2` runs two threads that each raise their own
   counter and shows both counters rising while a third thread reads the
   clock — that is, two threads running at the same time and not in turn.
4. A kernel test shows a thread of priority higher than the one running
   on its home processor taking that processor after the IPI.
5. A machine with `-smp 1` passes `preemption.rs`, `ipc.rs` and
   `syscalls.rs` unchanged.
6. 6.6.7 of [document 6](06-testing-strategy.md) lists the `Processors`
   cases.

## 16.23 S10. Remote invalidation

Status: not built.
Depends on: S8.
Size: L.

### Needs (already built)

- `TlbControl`, a trait with two methods, which `kernel-mm` calls and
  never implements (`crates/kernel/hal-api/src/paging.rs`, line 51).
- `LocalTlb`, the one implementation, at six call sites in the kernel
  binary (`crates/kernel/hal-x86_64/src/paging.rs`, line 24).
- `AddressSpaceControl::activate`, the one place a page-table root is
  loaded (`crates/kernel/hal-x86_64/src/paging.rs`, line 108).

### Does

1. Each `Processor` gains two atomic words: the page-table root it has
   loaded, written by `activate`, and a request word holding the page to
   invalidate, a generation counter, and an acknowledgement counter.
   Both are atomics and not fields of a cell, because `SharedTlb` is
   handed to `kernel-mm` beside the machine cell and cannot borrow the
   cell a second time.
2. Add `SharedTlb`, a second implementation of `TlbControl`. Its
   `flush_page` invalidates on the calling processor, then, for every
   other processor whose published root is this address space, writes the
   request, sends `INVALIDATE`, and waits for the acknowledgement. Its
   `flush_all` does the same with a whole-space request.
3. Replace `LocalTlb` with `SharedTlb` at the six call sites. `LocalTlb`
   stays for the bring-up, which runs before any other processor exists.
4. Add `remote::poll`: perform the pending request of the calling
   processor and raise its acknowledgement counter. It takes no
   kernel cell.
5. The handler of `INVALIDATE` acknowledges at the local APIC and calls
   `remote::poll`.
6. The kernel's `ExclusiveToken::wait` calls `remote::poll`, which is
   what D6 needs: a processor waiting for a cell with interrupts off
   still answers an invalidation.

The cost is O(processors sharing the address space) messages per changed
page, and the sender waits for all of them. An unmap of a range therefore
costs one round of messages per page; the bound `MAX_PAGES_PER_CALL`
already caps how many pages one system call changes.

The wait is bounded for each of the three states a target can be in. A
target in user mode takes the interrupt at once. A target waiting for a
kernel cell polls, which D6 requires. A target inside the kernel with
interrupts off and no wait answers when it returns to user mode through
`iretq`, which restores the flag from its own frame, and every system
call is bounded (2.5.4 of [document 2](02-architecture.md)).

### Produces

An address space that is safe to change while another processor
translates through it, which is what every `memory_unmap` and every
process teardown needs.

### Done when

1. A kernel test with `-smp 2` maps a page in a process whose two threads
   are on two processors, has both read it, unmaps it, and has the second
   processor's thread fault — not read the old bytes.
2. A kernel test shows the acknowledgement counter rising once per
   request per processor.
3. A kernel test shows a processor waiting for the machine cell
   answering an invalidation, which is the cycle D6 names.
4. A machine with `-smp 1` sends no message, and `memory.rs` passes
   unchanged.

## 16.24 S11. Measurement, and what it decides

Status: not built.
Depends on: S9, S10.
Size: M.

### Needs (already built)

- `bench.rs`, which measures a system call round trip and an IPC round
  trip in time-stamp counter ticks, and whose figures D-102 was decided
  from (`crates/kernel/bin/tests/bench.rs`).

### Does

Measure four numbers, each on as many processors as the number is defined
for:

| # | Measurement | Why it decides something |
|---|-------------|--------------------------|
| 1 | The `yield` round trip, on one and on four processors | It is the shortest path through the machine cell, so it is what the wait costs an uncontended caller. |
| 2 | The call-and-reply round trip between two threads on two processors | It is the same path with an IPI in it. |
| 3 | The share of time a processor spends waiting for the machine cell, counted in the wait itself, on two and on four processors | It is the number that says whether per-object locks are worth building. |
| 4 | An unmap of one page shared by four processors | It is what remote invalidation costs. |

### Produces

The decision D-17 defers: whether the machine cell splits into per-object
locks. The rule is stated before the numbers are read, so that the
numbers decide it: the cell splits when measurement 3 is above a quarter
on four processors, and stays whole otherwise.

### Done when

1. The four numbers stand in the decision register with the machine they
   were measured on.
2. The pointer 8.18 of [document 8](08-roadmap.md) holds to this document
   becomes a row of the phase overview in 8.1.
3. 2.5.3 and 2.5.4 of [document 2](02-architecture.md) describe what was
   built and not what was planned.
4. `sh tools/xtask-check.sh` exits with 0 on a run with `-smp 1` and on a
   run with `-smp 4`.

## 16.25 Risks

| # | Risk | Effect | What reduces it |
|---|------|--------|-----------------|
| 1 | No usable frame below 1 MiB. | No application processor starts. | S7 step 1 searches the map rather than fixing an address, and the machine boots on one processor and says so. The start-up code is position-independent (D2, reason 2), so any low frame serves. |
| 2 | An application processor faults before it reaches Rust. | It triple-faults; `-no-reboot` turns that into a QEMU exit, which the runner reports as a crash of the whole machine. | S7 ends with a test that reports the second processor's own reading of its identifier, so the first thing built is the thing that proves the sequence. The boot processor's wait has a deadline, so a processor that never reports costs 100 ms and not the run. |
| 3 | The wait of S5 replaces a refusal that some caller relied on. | A path that used to skip now blocks. | D5 keeps the refusal for the processor that holds the cell, which is every caller that exists today. S5's Miri test insists on the refusal. |
| 4 | Two processors take two cells in opposite orders. | Both wait forever. | D6 fixes the order and names the four sites that already have it. A fifth cell is a change to D6. |
| 5 | The `-smp 4` run under TCG is slower and finds new flakes. | The check takes longer and fails intermittently. | `-smp 1` stays the default of every existing run (S2). A run with more processors is the acceptance of this track and not of everything else. |
| 6 | Round robin gives one processor the busy threads. | One processor is loaded and another idles. | D4 states the cost and names work stealing as what removes it, after S11 has measured. |
| 7 | Remote invalidation is missed on a path that changes a page table without `TlbControl`. | A processor reads memory through a translation that no longer exists. | R5 of [document 4](04-safety-policy.md) keeps every page-table change in `kernel-mm`, which reaches the hardware only through `TlbControl`, `FrameAccess` and `activate`. S10 replaces the implementation at all six call sites and leaves `LocalTlb` reachable only from the bring-up. |
| 8 | A user process has two threads on two processors and shares state between them. | Its own data races, in userland. | The userland runtime holds no `static` and each thread gets its own IPC buffer. What a program shares, it shares through a memory object it asked for, which is the program's business and not the kernel's. |
| 9 | The calibration of two processors runs at once. | Channel two of the interval timer is driven by two callers and both read nonsense. | S7 step 9 starts one processor at a time. |
| 10 | An invalidation reaches a processor that is inside the kernel with interrupts off and is waiting for nothing. | The sender waits until that processor returns to user mode. | Every system call is bounded (2.5.4 of [document 2](02-architecture.md)), so the wait is bounded by one system call. S10 states the three states a target can be in and what each costs. |
