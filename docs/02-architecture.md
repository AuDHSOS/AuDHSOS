# 2. Architecture

This document describes the target design of the loader, the kernel, and
the userland. Binding decisions are listed in the
[decision register](09-decisions.md).

## 2.1 Overview

```
 ring 3 │ ┌──────────┐ ┌───────────┐ ┌────────────┐ ┌───────────┐ ┌───────┐
        │ │ root task│ │name server│ │console drv.│ │memory srv.│ │ apps  │
        │ └────┬─────┘ └─────┬─────┘ └─────┬──────┘ └─────┬─────┘ └───┬───┘
        │      └─────────────┴──── IPC: endpoints, notifications ──────┘
 ═══════╪════════════ system calls: handle + arguments in IPC buffer ═══════
 ring 0 │ ┌────────────────────────────────────────────────────────────────┐
        │ │ kernel (safe Rust): objects · handles · address spaces ·       │
        │ │ threads · scheduler · IPC · interrupt forwarding · syscalls    │
        │ ├────────────────────────────────────────────────────────────────┤
        │ │ HAL adapter x86_64 (unsafe, inline asm): privileged registers, │
        │ │ descriptor tables, MMU commit, APIC, timer, UART, QEMU exit,   │
        │ │ context switch                                                 │
        │ └────────────────────────────────────────────────────────────────┘
 ═══════╪══════════════════════════════════════════════════════════════════
 boot   │ loader (own UEFI application): reads kernel and boot image from  │
        │ the disk, builds page tables with the kernel's mapper, enters    │
        │ the kernel with the boot information structure                   │
 ═══════╪══════════════════════════════════════════════════════════════════
 QEMU   │ q35 machine, UEFI firmware bundled with QEMU
```

Three properties define the design:

1. **The kernel has no heap.** All kernel state lives in typed pools that are
   sized once at boot from the kernel reserve. Every allocation is fallible
   and returns an error to the caller.
2. **The kernel never dereferences user pointers.** System calls exchange
   data only through fixed-layout IPC buffers, never through registers or
   user pointers; the kernel reaches the buffers through its physical
   memory window.
3. **Every algorithm is written against a trait.** The page-table walker,
   the frame allocator, the scheduler, the IPC state machine, the ELF parser,
   and the UART register logic run unchanged on the host under test, in the
   loader, in the kernel, and in userland. Only the adapters differ.

## 2.2 What runs where

| Responsibility | Kernel part | Userland part |
|----------------|-------------|---------------|
| Page tables | create, map, unmap, protect, activate | decides what is mapped where, when, and for whom |
| Threads and scheduling | thread state, context switch, fixed-priority run queues | creates threads, assigns priorities |
| IPC | copies between IPC buffers, installs handles, delivers badges | protocols, servers, clients |
| Capabilities | handle tables, rights checks, quotas | delegation of authority |
| Interrupts | masks the line, sends end-of-interrupt, signals the bound notification | services the device, acknowledges the interrupt |
| Physical memory | kernel reserve and object pools at boot; hands every other frame to the root task as memory objects | allocation policy, zeroing, accounting (memory server) |
| Debug UART | build-time feature for kernel diagnostics; absent in release builds | - |
| Console and every other device | provides port, device memory, and interrupt capabilities | drivers |
| Graphics and input (Phases 9 to 11) | hands out the framebuffer as a `Device` memory object and the i8042 ports and interrupt lines as capabilities | display server, input driver, applications |
| Program loading | validates the eight fields of the boot image header; maps the root task | tar reader, ELF loader, process creation |
| Naming | - | name server |
| Faults | converts a fault into a message to the fault handler endpoint | fault handler decides: repair, resume, kill |
| File systems, networking, time of day | - | servers in later phases |

## 2.3 Kernel object model

### 2.3.1 Object types

| Object | Purpose | Operations | Rights |
|--------|---------|------------|--------|
| `Process` | Address space, handle table, threads, quotas, fault handler | create, install handle, set fault handler, map/unmap/protect memory, kill | `MANAGE`, `MAP`, `INSTALL` |
| `Thread` | Execution context inside a process | create, start, suspend, resume, set priority, kill, exit (self) | `MANAGE` |
| `MemoryObject` | Contiguous range of physical frames; kind `Ram` or `Device` | split, map, query info | `READ`, `WRITE`, `EXECUTE`, `MAP`, `INFO` |
| `Endpoint` | Synchronous rendezvous point | call, send, recv, reply-recv, badge | `SEND`, `RECV`, `BADGE` |
| `Reply` | One-shot right to answer a specific caller; created by `recv` | reply | implicit |
| `Notification` | 64 signal bits | signal, wait, poll, bind interrupt | `SIGNAL`, `WAIT`, `BIND` |
| `Interrupt` | A bound hardware interrupt line | bind to notification, acknowledge | `MANAGE` |
| `IoPortRange` | Permission for a range of x86 I/O ports | read, write | `READ`, `WRITE` |
| `SystemControl` | Root authority to create interrupts, port ranges, device memory objects | create interrupt, create port range, create device memory object, query system info | `MANAGE` |

Every object type also carries the generic rights `DUPLICATE` and
`TRANSFER`.

### 2.3.2 Handles and rights

- A handle is a `NonZeroU64`. The low 32 bits index the handle arena the
  machine shares; the high 32 bits are a generation counter that changes
  every time the slot is reused. A slot names the process it belongs to, and
  a lookup checks that owner as well as the generation, so one process
  cannot name a slot of another (D-58). A stale handle fails with
  `InvalidHandle`. Freed slots are reused in FIFO order. The `Handle` newtype and the index and
  generation widths are defined once, in `audhsos-abi`; no code treats a
  handle as a bare integer.
- Handle `0` is never valid.
- Every process has a handle capacity fixed at its creation, chosen by the
  creator within its own quota. The capacity is a ceiling the quota enforces
  against the shared arena, not memory set aside for the process.
- `handle_duplicate(handle, rights)` succeeds only if `rights` is a subset of
  the current rights and the handle carries `DUPLICATE`.
- `handle_close(handle)` releases the slot and the reference the handle
  held. Closing the last handle to an object destroys the object; a process
  and a thread are the exception named in 2.3.4 (D-86).
- `Rights` is a project-defined bit set type with constant-time subset
  checks; every right has a fixed bit position listed in `audhsos-abi`.
- A badge is attached to an endpoint capability with `endpoint_badge`. The
  result carries `SEND` only and cannot be re-badged. A receiver sees the
  badge of the capability the sender used.

### 2.3.3 Object storage and identity

Objects live in one pool per object type. A pool entry has a generation
counter and a reference count. The kernel refers to an object by a typed id
(`ObjectId<T>` = index plus generation). Every id lookup checks the
generation. Pools are plain arrays in safe Rust and are tested on the host.

### 2.3.4 Lifetime

- An object is destroyed when its reference count reaches zero. References
  come from handles, from mappings, and from bindings. A thread blocked on
  an object holds no reference to it, so closing the last handle to an
  endpoint destroys it while threads still wait on it (D-75).
- A process and a thread hold one further reference: their own (D-86). A
  process ends when it is killed and a thread when the kernel has given back
  what it held, whatever handle still names either of them, and closing the
  last handle to a running thread therefore does not end it. The handles that
  named one afterwards name nothing, which is what a stale handle is.
- Destroying an endpoint or notification wakes every blocked thread with
  `ObjectDestroyed`. Dropping a reply object without replying wakes its
  caller with `ReplyDropped`.
- Destroying a process kills its threads, drops its mappings, and closes its
  handle table, and every handle it closes releases the reference that handle
  held.
- Destroying a `Ram` memory object does not return frames to the kernel. The
  memory server keeps a handle to every object it hands out. The kernel
  makes no promise about the contents of memory. The memory server
  overwrites every memory object with zeros before handing it out and again
  immediately when it is returned.

### 2.3.5 Revocation

There is no recursive revocation in the first release. A server that must
cut off a client hands that client a badged endpoint and stops serving that
badge.

## 2.4 Memory

### 2.4.1 Physical memory

1. The loader passes the boot information structure with a list of memory
   regions. A pure function normalizes it: sorts regions, merges adjacent
   usable regions, removes overlaps with reserved regions, keeps the kernel
   image, the boot image, the page tables, and the boot stack out of the
   usable set, and aligns every region to frame boundaries.
2. The kernel takes the kernel reserve from the normalized map: a size
   computed from total RAM (default: 1/16 of RAM, at least 4 MiB, at most
   64 MiB, overridable in the boot image header). The reserve holds what the
   count of is not known before boot: page tables, kernel stacks, and the
   IPC buffers of threads. The object pools are `static` cells and live in
   the `.bss` of the kernel image (D-57). A bitmap frame allocator manages
   the reserve, over at most 16384 frames, so the reserve never exceeds
   64 MiB whatever the machine has.
3. Every remaining usable region becomes one `Ram` memory object owned by the
   root task. From then on the kernel allocates user memory never again.

### 2.4.2 Memory objects and mappings

- A memory object is a contiguous physical range `[start, end)` in whole
  frames, with a kind (`Ram` or `Device`) and, for device memory, a cache
  policy.
- `memory_split(handle, offset)` turns one object into two adjacent ones. The
  original handle refers to the lower part; a new handle for the upper part
  is returned. The kernel never merges objects.
- `memory_map(process, memory, vaddr, offset, len, permissions)` populates
  page tables immediately. Page-table frames come from the kernel reserve
  and count against the process's kernel-object quota.
- There is no demand paging in the kernel. A page fault becomes a fault
  message; the fault handler may map memory and resume the thread.
- `memory_info(handle)` returns the physical start address and length and
  requires the `INFO` right.

### 2.4.3 Address spaces

An address space is a page-table root plus a fixed-capacity sorted array of
`Region { start, len, memory: ObjectId, offset, permissions }`. Invariants:

- Regions do not overlap.
- Every region lies inside the user half of the address space
  (`0x0000_0000_0001_0000 ..= 0x0000_7FFF_FFFF_FFFF`). The lowest 64 KiB is
  never mappable.
- Start and length are page-aligned and `start + len` does not overflow.
- Unmapping the middle of a region splits it into two regions; the region
  quota is checked before the split happens.

The kernel address space (upper half) is managed by the same code. Layout:

| Range (virtual) | Content |
|-----------------|---------|
| Physical memory window | All of RAM mapped read/write, no-execute, at `PHYS_WINDOW_BASE`; the kernel maps the device apertures it drives into the same range at the same offset, uncached (D-60) |
| Kernel image | Text read-only/execute, rodata read-only, data read/write, at `KERNEL_BASE` |
| Boot information page | The structure the loader wrote, read-only, at `BOOT_INFO_VADDR` |
| Boot stack | `BOOT_STACK_PAGES` pages read/write, no-execute, ending at `BOOT_STACK_TOP`, with one unmapped guard page below |
| Kernel stacks | `KERNEL_STACK_SLOTS` slots at `KERNEL_STACKS_BASE`, each slot one unmapped guard page followed by `KERNEL_STACK_PAGES` mapped pages the stack grows down through |
| Per-CPU area | Current thread pointer, scratch space (one CPU in the first release) |

`PHYS_WINDOW_BASE`, `KERNEL_BASE`, `BOOT_STACK_TOP`, `BOOT_STACK_PAGES`,
`BOOT_INFO_VADDR`, `KERNEL_STACKS_BASE`, `KERNEL_STACK_PAGES`,
`KERNEL_STACK_SLOT_PAGES`, and `KERNEL_STACK_SLOTS` are constants in
`audhsos-abi` shared by the loader and the kernel. `MAX_PHYS_WINDOW_BYTES`
is the memory the window covers before it would reach the stack area; a
machine with more memory is refused at boot.

An address space belongs to a process and lives in the `Process` object:
the page-table root and the region table are fields of it, there is no
address-space object and no id for one (D-65). The kernel half is shared by
copying page-map level four entries into every new root: the window stays
inside entry 256, and the kernel stacks, the boot information page, and the
image are all inside entry 511. Both entries exist once the memory bring-up
is done, so a kernel stack allocated later changes only tables below entry
511 and is visible in every address space at once.

The frames of the reserve hold the page tables, the kernel stacks, and the
IPC buffers of threads. The object pools of Phase 5 are `static` cells in
the `.bss` of the kernel image, sized by the constants in
`kernel-core::config` (D-57). Their empty state is all zeros and their
constructors are `const`, so a pool is never built on the boot stack and
never moved into its cell (D-66).

### 2.4.4 Memory management in safe Rust

Logic operates on typed values; only the final commit to the hardware is an
adapter.

| Concern | Safe logic (host-tested) | Adapter (unsafe) |
|---------|--------------------------|------------------|
| Addresses | `PhysAddr`, `VirtAddr`, `PhysFrame`, `Page`, ranges; constructors validate alignment and canonical form and return `Result` | none |
| Memory map | normalization, reserve selection | none |
| Frame allocation | bitmap allocator, contiguous first-fit | none |
| Page-table entries | `PageTableEntry(u64)` with typed flags, encode/decode, reserved-bit checks | none |
| Page-table walk | `Mapper<A: FrameAccess, T: TlbControl, S: FrameSource>`: creates intermediate tables, maps, unmaps, frees empty tables, reports which pages need flushing | kernel: `FrameAccess` through the physical window, `TlbControl` with `invlpg`, `activate` writes `CR3`; loader: `FrameAccess` through the identity mapping, `TlbControl` as a no-op |
| IPC buffer access | message encode/decode over `&mut [u8; 4096]` | `PhysicalWindow::frame_bytes_mut(frame)` |
| Kernel stacks | stack pool bookkeeping, initial frame layout for a new thread written as `u64` values into a `&mut [u64]` | stack pointer switch |

Under test, `FrameAccess` is a `HashMap<PhysFrame, Box<PageTable>>`,
`TlbControl` records the flush requests, and `FrameSource` is a counter.
There is one implementation of every algorithm.

### 2.4.5 Faults

- A fault in user mode (page fault, general protection, invalid opcode,
  divide error, breakpoint, alignment check) becomes an IPC call from the
  faulting thread to the process's fault handler endpoint. The message
  carries the fault kind, faulting address, instruction pointer, and error
  code. The handler replies to resume the thread, or kills it.
- If a process has no fault handler, the thread enters state `Faulted`; the
  process's creator inspects it with `thread_info`.
- A fault in kernel mode prints diagnostics on the debug UART (if compiled
  in), exits QEMU with the failure code (if compiled in), and otherwise
  halts.
- Double faults run on a separate interrupt stack (IST).

## 2.5 Threads and scheduling

### 2.5.1 Thread control block

Stored in the thread pool: process id, state, priority, remaining time
slice, kernel stack id, IPC buffer frame, IPC state, fault information,
queue links (indices), and the saved context, which is one word: the kernel
stack pointer of the thread while it is not running, written and read
through the HAL trait `Context` (D-67).

### 2.5.2 States

| State | Meaning | Leaves the state through |
|-------|---------|--------------------------|
| `Inactive` | created, not started | `thread_start` |
| `Ready` | in a run queue | scheduler picks it |
| `Running` | on the CPU | block, preemption, exit |
| `BlockedSend` | waiting for a receiver on an endpoint | rendezvous, object destroyed, kill |
| `BlockedRecv` | waiting for a sender on an endpoint | rendezvous, object destroyed, kill |
| `BlockedReply` | waiting for the reply to a call | reply, reply object dropped, kill |
| `BlockedNotification` | waiting for signal bits | signal, object destroyed, kill |
| `Suspended` | stopped by `thread_suspend` | `thread_resume`, kill |
| `Faulted` | stopped after a fault with no handler | `thread_resume` after repair, kill |
| `Exited` | finished; slot released when the last reference drops | - |

Transitions are a table in code; every illegal transition is an error, not
a panic.

### 2.5.3 Scheduler

- 32 fixed priorities. Each priority has a FIFO run queue built from indices
  into the thread pool. Higher priority always preempts lower priority.
  Equal priorities share the CPU round-robin with a time slice measured in
  timer ticks.
- The idle thread has the lowest priority and never blocks. Its kernel stack
  is the boot stack the loader provided.
- A thread may only create threads with priority up to its own maximum
  priority, which the creator sets at creation.
- Interface (pure logic): `enqueue`, `dequeue`, `pick_next`, `tick`,
  `yield_now`, `set_priority`, `on_block`, `on_wake`.
- The first release has one CPU. The SMP path is fixed: per-CPU run queues,
  a per-CPU current-thread pointer, a big kernel lock first, per-object
  locks only after measurement.

### 2.5.4 Kernel execution model

The kernel is non-preemptible. Interrupts are disabled while the kernel
runs. Every system call is bounded: operations over ranges process at most
a fixed number of pages per call and return `Partial` with a progress count
so that userland loops.

### 2.5.5 Context switch and entry paths

- Every thread has a kernel stack. An interrupt or system call from user mode
  lands on the current thread's kernel stack (`RSP0` in the TSS).
- Preemption switches kernel stacks inside the timer interrupt handler. The
  switch saves callee-saved registers and the stack pointer of the outgoing
  thread and restores those of the incoming thread. This is the single
  context-switch routine and is a naked function in the HAL adapter.
- A newly created thread starts with a synthesized interrupt frame on its
  kernel stack, so the first switch into it "returns" into user mode at the
  entry point. The frame is written as plain `u64` values in safe Rust.
- The page-table root is loaded through the HAL trait `AddressSpaceControl`
  and only when the incoming thread belongs to another process, because
  writing `CR3` costs the translation lookaside buffer (D-65). Threads of
  one process switch without touching it.

## 2.6 IPC

### 2.6.1 Endpoint operations

| Operation | Semantics |
|-----------|-----------|
| `ipc_call(endpoint)` | Sends the message in the IPC buffer and blocks until the receiver replies. Send and wait-for-reply are atomic from the receiver's point of view. |
| `ipc_send(endpoint)` | Sends and blocks until a receiver has taken the message. No reply. |
| `ipc_recv(endpoint)` | Blocks until a sender arrives. Returns the message, the badge, and a `Reply` handle if the sender used `call`. |
| `ipc_reply(reply)` | Delivers the message in the IPC buffer to the caller and consumes the reply object. |
| `ipc_reply_recv(reply, endpoint)` | `reply` followed by `recv` without returning to userland in between. |
| `ipc_try_recv(endpoint)` | Like `recv` but returns `WouldBlock` instead of blocking. |

Queueing: senders wait in a queue on the endpoint ordered by priority, then
FIFO. Receivers wait in a second queue. A rendezvous happens as soon as both
queues are non-empty. Killing a thread removes it from any queue.
Destroying the endpoint wakes all waiters with `ObjectDestroyed`. Dropping
a `Reply` object without replying wakes the caller with `ReplyDropped`.

### 2.6.2 Message layout

The message is a region of the IPC buffer:

| Field | Size | Meaning |
|-------|------|---------|
| label | 8 bytes | protocol-defined tag |
| word count | 8 bytes | number of payload words, `0..=MAX_WORDS` |
| handle count | 8 bytes | number of handles, `0..=4` |
| handles | 4 × 8 bytes | handles to transfer; must carry `TRANSFER`; copied into the receiver's table with the same rights; the sender keeps its handles |
| words | up to 480 × 8 bytes | payload |

The kernel copies exactly `word count` words from the sender's IPC buffer to
the receiver's IPC buffer and installs the handles. If the receiver's handle
table is full, the message is still delivered with `handle count` set to the
number that fit and an error flag in the receiver's result. Bulk data goes
through shared memory objects.

### 2.6.3 Notifications

- `notification_signal(handle, bits)` ORs `bits` into the word. Signalling
  zero is a no-op that succeeds.
- `notification_wait(handle)` blocks until the word is non-zero, then
  returns and clears it.
- `notification_poll(handle)` returns and clears without blocking.
- Several signals before a wait are merged. Only one thread may wait on a
  notification at a time; a second waiter gets `Busy`.
- An `Interrupt` object binds to a notification with a bit index. The kernel
  signals that bit on each interrupt.

### 2.6.4 Fault and startup messages

- Fault messages are kernel-generated `call`s with a reserved label range.
  The reply resumes the thread.
- When a process is created, its creator writes a startup message into the
  first thread's IPC buffer before `thread_start`. It lists the initial
  handles and their meaning. The kernel does not interpret this message.

## 2.7 Interrupts and devices

- The root task holds the `SystemControl` capability and creates `Interrupt`
  objects for specific lines, `IoPortRange` objects, and `Device` memory
  objects for MMIO regions. It hands them to drivers.
- Interrupt flow: line asserts → kernel masks the line at the I/O APIC and
  sends end-of-interrupt to the local APIC → kernel signals the bound
  notification → driver thread wakes, services the device →
  `interrupt_ack(handle)` unmasks the line.
- I/O port access is a system call on an `IoPortRange` handle in the first
  release. Userland drivers contain no inline assembly.
- DMA: a driver receives a `Ram` memory object with the `INFO` right and
  programs the physical address into the device. No IOMMU support in the
  first release.

## 2.8 System call interface

- Entry: software interrupt vector `0x80` with descriptor privilege level 3.
  The handler is an `x86-interrupt` ABI function.
- Arguments: the system call number, up to six argument words, and the
  message region live at fixed offsets in the calling thread's IPC buffer.
  Results (status and up to two return words) are written to the same
  buffer. Registers carry nothing.
- Validation order for every call: number known → handle valid → object type
  matches → rights sufficient → arguments valid → quota available. The first
  failing check determines the error.
- Errors are an exhaustive `enum Error` in `audhsos-abi` with a stable
  numeric representation.
- The system call table is one declarative macro in `audhsos-abi`. It
  generates numbers, names, argument counts, the kernel dispatcher, and the
  userland wrappers.
- The `syscall` instruction path with register arguments is evaluated in
  Phase 8 by measurement.

| Call | Object | Purpose |
|------|--------|---------|
| `process_create` | Process (creator) | new process with handle-table and region quotas, kernel-object quota |
| `process_install_handle` | Process | copy a handle of the caller into the target process, returning the child's handle number |
| `process_set_fault_handler` | Process | set the endpoint that receives fault messages |
| `process_kill` | Process | terminate |
| `thread_create` | Process | new thread: entry, stack pointer, IPC buffer (memory object, offset), priority, maximum priority |
| `thread_start`, `thread_suspend`, `thread_resume`, `thread_kill` | Thread | state changes |
| `thread_set_priority` | Thread | change priority within the maximum |
| `thread_info` | Thread | state and fault information |
| `thread_exit`, `thread_yield` | self | no handle |
| `memory_split` | MemoryObject | split at offset |
| `memory_map`, `memory_unmap`, `memory_protect` | Process + MemoryObject | mappings; bounded per call |
| `memory_info` | MemoryObject | physical range (requires `INFO`) |
| `handle_duplicate`, `handle_close` | any | handle table operations |
| `endpoint_create`, `endpoint_badge` | Process / Endpoint | create; derive a badged send-only capability |
| `ipc_call`, `ipc_send`, `ipc_recv`, `ipc_try_recv`, `ipc_reply`, `ipc_reply_recv` | Endpoint / Reply | messaging |
| `notification_create`, `notification_signal`, `notification_wait`, `notification_poll` | Notification | signals |
| `interrupt_create`, `interrupt_bind`, `interrupt_ack` | SystemControl / Interrupt | interrupt forwarding |
| `ioport_create`, `ioport_read`, `ioport_write` | SystemControl / IoPortRange | x86 port I/O |
| `memory_create_device` | SystemControl | device memory object |
| `system_info` | SystemControl | pool capacities and usage, tick frequency, and the address of the root system description pointer; the framebuffer description joins it in Phase 9 |
| `debug_log` | none | writes the message region to the debug UART; exists only in builds with the `debug-uart` feature |

## 2.9 Boot sequence

1. The UEFI firmware bundled with QEMU starts `EFI/BOOT/BOOTX64.EFI` from
   the EFI system partition of the disk image. That file is the loader, a UEFI application
   from this repository, running in 64-bit mode with identity-mapped memory.
2. The loader opens the boot volume and reads `AUDHSOS/KERNEL.ELF` and
   `AUDHSOS/BOOT.IMG` into pages allocated from the firmware.
3. The loader parses the kernel ELF with `audhsos-elf`, allocates frames for
   every load segment, copies the segment bytes, and zero-fills the rest.
4. The loader allocates frames for page tables, a boot stack with a guard
   page, and the boot information page. It builds the page tables with the
   `kernel-mm` mapper: the physical memory window at `PHYS_WINDOW_BASE`, the
   kernel segments at `KERNEL_BASE` with their ELF permissions, the boot
   stack, the boot information page, and an identity mapping of the
   loader's own image and current stack.
5. The loader reads the ACPI root pointer from the UEFI configuration table
   and the framebuffer description from the Graphics Output Protocol,
   retrieves the final memory map, and calls `ExitBootServices`.
6. The loader converts the UEFI memory map into the boot information
   structure (see [3.1.5](03-target-platform.md#315-boot-information-structure)).
   The loader's own image becomes usable memory; page tables, boot stack,
   kernel, and boot image are marked with their own kinds.
7. The loader's naked entry function writes the page-table root to `CR3`,
   switches to the boot stack, and jumps to the kernel entry point with the
   address of the boot information structure. The loader never regains
   control.
8. The kernel entry (`extern "C" fn(*const BootInfo) -> !` in the kernel
   binary) hands the pointer to the HAL adapter, which validates magic,
   version, size, and region count and produces a `&BootInfo`. The boot
   information names its own physical address in no field, so the adapter
   walks the loader's tables for `BOOT_INFO_VADDR` and appends the result
   as the region of kind `BootInfo`. Control passes to
   `kernel_core::boot`, which is architecture neutral.
9. HAL initialization: GDT with kernel and user segments, TSS with a
   double-fault stack, IDT with a gate for every exception the processor
   defines, for every device vector of the plan, and from Phase 5 for
   vector `0x80`; debug UART if compiled in. Interrupts stay off.
10. Memory: normalize the memory regions, take the kernel reserve in the
    size the boot image header asks for, initialize the frame allocator,
    the kernel region table, and the kernel stack pool, adopt the loader's
    page tables into the kernel's address-space bookkeeping by walking the
    four fixed ranges of the kernel half, and drop the loader's identity
    mapping without giving a frame back. The object pools follow in
    Phase 5 with the types they hold.
11. Interrupts: read the root pointer, the root table, and the MADT through
    the physical window; map the register window of the local APIC and of
    every I/O APIC into the physical window, uncached (D-60); move the two
    legacy controllers to the vectors of the plan and mask them; turn the
    local APIC on with the spurious vector; mask every I/O APIC line;
    measure the local APIC timer against channel two of the interval timer
    and start it at `TICKS_PER_SECOND`; turn interrupts on. Interrupts come
    after memory because the register windows are mapped out of the address
    space the memory bring-up leaves.
12. Boot image: validate the header. Create a `Ram` memory object for the
    image.
13. Root task: create the process with maximum quotas; map the root task at
    `ROOT_TASK_BASE`; create an IPC buffer and a stack; install handles for
    `SystemControl`, the process itself, the boot-image memory object, and
    one memory object per free RAM region; write the startup message; start
    the thread.
14. Idle thread on the boot stack; scheduler starts. From here on the kernel
    only reacts to interrupts and system calls.
15. The root task parses the tar archive, loads the name server, the console
    driver, and the memory server, and grants each its capabilities.

## 2.10 Userland

| Crate | Content | `unsafe` |
|-------|---------|----------|
| `user-sys-x86_64` | `_start`, the `int 0x80` wrapper, the `GlobalAlloc` adapter that turns allocator offsets into pointers | allowlisted |
| `user-rt` | typed handle newtypes with `Drop`, system call wrappers over the IPC buffer, message builder and parser, safe offset-based heap allocator, panic handler that reports over a log endpoint and exits, logging macros | no |
| `user-proto` | message encodings for the name and console protocols, versioned labels | no |
| `user-loader` | tar (ustar) reader; process creation from an ELF using `audhsos-elf` | no |
| `server-init` | the root task: boot image parsing, starting servers, distributing capabilities | no |
| `server-name` | registry: `register(name, endpoint)`, `lookup(name)`, with badge-based ownership | no |
| `server-console` | 16550 UART driver: `driver-uart16550` register logic over `IoPortRange` system calls plus an `Interrupt`; `write(bytes)`, `read(max)` | no |
| `server-memory` | allocation policy over memory objects: `allocate(len, alignment)`, `release`; zeroes every object before hand-out and immediately after return | no |
| `app-hello` | end-to-end demonstration and test client | no |
| `gfx` (Phase 9) | framebuffer logic: pixel formats, filling, blitting, clipping, damage rectangles, the project's bitmap font, text rendering | no |
| `server-display` (Phase 9) | owns the framebuffer `Device` memory object; surfaces backed by shared memory objects, `present` with damage rectangles, cursor | no |
| `driver-i8042` (Phase 10) | i8042 controller and PS/2 device logic over the port access trait: controller initialization, scancode set 2 decoding, mouse packet parsing | no |
| `server-input` (Phase 10) | owns the i8042 port range and the interrupts for lines 1 and 12; delivers key and pointer events to subscribers through a ring buffer in a shared memory object plus a notification | no |
| `app-canvas` (Phase 11) | graphical demonstration and end-to-end test client: cursor, drawing, typed text | no |

Process creation from userland: `process_create` → for each ELF segment,
allocate memory from the memory server, map it into the loader's own address
space, copy the bytes, unmap, then map it into the child with the segment's
permissions → allocate stack and IPC buffer → `thread_create` →
`process_install_handle` for every initial capability → write the startup
message into the child's IPC buffer → `thread_start`.

## 2.11 Security model

- Capabilities are the only source of authority. There is no ambient
  authority, no global namespace in the kernel, and no way to name an object
  without a handle to it.
- Rights only decrease along duplication and transfer.
- The root task starts with everything and gives each server the minimum it
  needs.
- The kernel does not trust message contents. Sender identity is the badge.
- Kernel-object quotas prevent one process from exhausting the pools of
  another.
- No code outside this repository runs in the loader, the kernel, or the
  userland.
