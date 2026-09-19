# kernel-objects and kernel-types audit findings

Repository: AuDHSOS/AuDHSOS. Audit of kernel-objects and kernel-types at main bc47df5.
Each `## ` section is one issue, filed 2026-09-19 with the number in its heading. `Title:` is the issue title, `Labels:` the labels, everything after `Body:` is the issue body verbatim.

---

## F01 — issue #73
Title: kernel-objects: an empty pool is not all zeros, so the machine cell lands in `.data` and not in `.bss`
Labels: bug, part::kernel
Body:
`Slot<T>` stores its occupant as `Option<Occupant<T>>` (`crates/kernel/objects/src/pool.rs:229-233`). The compiler encodes `None` through a niche of `T` wherever `T` has one, and every object type except `IoPortRange` has one: `Process::ended` and `RegionTable::kernel` are `bool`, `Thread::state` is an enum, `MemoryObject::kind` is an enum, `Endpoint`, `Notification`, and `Interrupt` hold `Option` tags, `Reply::consumed` is a `bool` (`crates/kernel/objects/src/object.rs:135-142, 189-216, 362-415, 497-502, 531-537, 560-565, 594-610`). For each such `T` the `None` of `Slot::FREE` (`pool.rs:239-243`) is a byte of value 2 at the niche's offset, not zero. The doc comments claim the opposite at `pool.rs:114-119`, `pool.rs:225-227`, `pool.rs:238`, `pool.rs:263-266`, `crates/kernel/objects/src/store.rs:6-9`, `store.rs:78-79`, `crates/kernel/core/src/machine.rs:9-11`, `docs/02-architecture.md:230-234`, and `docs/09-decisions.md:76` (D-66).

The built kernel shows the effect: `nm -S target/x86_64-unknown-none/debug/audhsos-kernel` lists `kernel_core::machine::MACHINE` (`machine.rs:43`) with symbol type `D` and size `0x15c518` (1,426,712 bytes), and `readelf -S` gives `.data` a size of `0x15f020` and `.bss` a size of `0x5158`. A host binary with one `static mut` per pool places `Pool<Process, 4>`, `Pool<Thread, 4>`, `Pool<MemoryObject, 4>`, `Pool<Endpoint, 4>`, `Pool<Notification, 4>`, `Pool<Reply, 4>`, and `Pool<Interrupt, 4>` in `.data` and `Pool<IoPortRange, 4>`, `HandleArena<4>`, and `Scheduler` in `.bss`. The kernel image file carries 1.36 MiB of pool bytes that the loader copies into memory, which is what D-66 was decided to prevent.

Fix: replace `Option<Occupant<T>>` in `Slot<T>` with a two-variant enum under `#[repr(u32)]` (`Free`, `Held(Occupant<T>)`), whose tag has a fixed offset and no niche, so `Free` is the zero word and the payload bytes stay unconstrained; the option not taken, an inline `Occupant<T>` with `refs == 0` meaning free, needs a zero value of every `T`.

---

## F02 — issue #74
Title: kernel-types: `VirtAddr::align_down` returns a non-canonical address for a kernel address and an alignment of 2^48 or above
Labels: bug, part::kernel
Body:
`VirtAddr::align_down` masks the raw word and wraps the result without a check (`crates/kernel/types/src/virt.rs:81-87`). Its doc comment states that the result stays in the same half "because both half boundaries are aligned to every alignment up to 2^47", but `Alignment::new` accepts every power of two up to 2^63 (`crates/kernel/types/src/align.rs:111-117`). The module invariant at `virt.rs:6` and the crate README (`crates/kernel/types/README.md:3-6`) state that a `VirtAddr` is always canonical.

`VirtAddr::KERNEL_MIN.align_down(Alignment::new(1 << 48)?)` yields `VirtAddr(0xFFFF_0000_0000_0000)`: `0xFFFF_8000_0000_0000 & !0xFFFF_FFFF_FFFF`, which `is_canonical` at `virt.rs:27-29` rejects. The value answers `false` to both `is_user` and `is_kernel` (`virt.rs:63-73`), and `VirtAddr` is the word the context switch writes through a pointer (`virt.rs:19-22`, D-67); a non-canonical word in a page-table walk or a saved context raises `#GP`. `VirtAddr::align_up` checks the same condition at `virt.rs:96-104`.

Fix: make `align_down` return `Result<VirtAddr, Error>` and reject a result that `is_canonical` rejects, as `align_up` does; the option not taken, capping `Alignment` at 2^47 in `Alignment::new`, also constrains `PhysAddr`, which needs no cap.

---

## F03 — issue #75
Title: kernel-objects: `config::THREADS` doc says a thread costs five frames, the layout and D-73 say nine
Labels: bug, part::kernel
Body:
The doc comment of `THREADS` states "Every thread costs five frames of the kernel reserve: four for its kernel stack and one for its IPC buffer (D-57)" (`crates/kernel/objects/src/config.rs:48-51`). `KERNEL_STACK_PAGES` is 8 (`crates/abi/src/layout.rs:75-81`), and D-73 records the change to nine frames per thread (`docs/09-decisions.md:83`).

A reader sizing the reserve from `config.rs` computes 1280 frames for 256 threads; the kernel takes 2304.

Fix: state nine frames, eight for the kernel stack and one for the IPC buffer, and cite D-73.

---

## F04 — issue #76
Title: kernel-objects: README says the pool sizes are decided outside the crate, `config` holds them
Labels: bug, part::kernel
Body:
The README states "its capacity is a const generic, so that the sizes are decided where the kernel is assembled and not here" (`crates/kernel/objects/README.md:3-6`). `crates/kernel/objects/src/config.rs:45-92` holds every size, its module doc says the numbers live in this crate and `kernel-core` re-exports them (`config.rs:41-43`), and `MachineObjects` at `crates/kernel/objects/src/store.rs:60-67` applies them.

A reader who changes a pool size in `kernel-core` following the README finds no size there.

Fix: state in the README that `config` holds the sizes and `MachineObjects` applies them, and that the const generic exists so a test can hold a small machine (`store.rs:24-27`).

---

## F05 — issue #77
Title: kernel-types: `PageRange::new` returns `Overflow` for a user range whose count overflows the addition, the doc promises `CrossesCanonicalHole`
Labels: bug, part::kernel
Body:
`PageRange::new` documents `CrossesCanonicalHole` for a user range that would reach the hole and `Overflow` for a kernel range that would wrap (`crates/kernel/types/src/virt.rs:255-258`). The match at `virt.rs:265-269` sends the `None` arm of `checked_add` to `Overflow` for both halves.

`PageRange::new(Page::containing(VirtAddr::new(0x1000)?), u64::MAX)` computes `1u64.checked_add(u64::MAX) == None` and returns `Error::Overflow` for a user start. A caller that maps the error variant to a user-facing error by half reports a wrong cause.

Fix: decide the variant by `start.is_user()` on both failing arms.

---

## F06 — issue #78
Title: kernel-objects: `WaitQueue::enqueue` accepts a thread that is the sole member of another queue
Labels: enhancement, part::kernel
Body:
`enqueue` and `requeue` reject a thread only when its `wait_links` are linked or when it is the head of this queue (`crates/kernel/objects/src/wait_queue.rs:88-90, 117-119`). `append` gives the sole member of a queue `Links { next: None, previous: None }` (`wait_queue.rs:205-227`), which `is_unlinked` reports as free. The doc comment promises `InvalidState` "when `id` is already in a queue" (`wait_queue.rs:79-81`).

Sequence on two queues A and B and threads t, u: `A.enqueue(t)`, `B.enqueue(t)` returns `Ok`, `B.enqueue(u)`, `A.dequeue_front()`. `unlink` at `wait_queue.rs:142-170` reads the links of t, which now belong to B, and sets `A.head = Some(u)` at `wait_queue.rs:159-161`, so A names a thread that waits in B. Safe today: `kernel-ipc` enqueues only the running thread (`crates/kernel/ipc/src/endpoint.rs:156, 217`) and requeues a peer whose `wait` it cleared with `forget` (`endpoint.rs:147, 203, 449-455`).

Fix: have `enqueue` and `requeue` refuse a thread whose `wait` (`crates/kernel/objects/src/object.rs:400-402`) is not `Wait::Nothing`; the callers set `wait` after enqueue (`endpoint.rs:156-157`) and clear it in `cancel` (`crates/kernel/ipc/src/cancel.rs:61`) and `forget`, so they need no change.

---

## F07 — issue #79
Title: kernel-objects: `WaitQueue::dequeue_front` leaves a head that names no live thread in place
Labels: enhancement, part::kernel
Body:
`dequeue_front` returns `None` when `unlink` fails (`crates/kernel/objects/src/wait_queue.rs:129-139`), and `unlink` fails before touching `head` when the pool does not hold the head (`wait_queue.rs:142-145`).

Sequence: `queue.enqueue(threads, t)`, `threads.force_release(t)` (`crates/kernel/objects/src/pool.rs:440-446`), `queue.dequeue_front(threads)`. The call returns `None`, `head` and `tail` stay `Some(t)`, `len` stays 1; every later `dequeue_front` returns `None`, and every later `enqueue` fails with `InvalidHandle` at `wait_queue.rs:216-222` because `threads.get_mut(t)` fails. Safe today: `kernel-syscall` calls `kernel_ipc::cancel` before every `force_release` of a thread that may wait (`crates/kernel/syscall/src/calls/thread.rs:451, 522`, `crates/kernel/syscall/src/calls/process.rs:354`).

Fix: have `dequeue_front` reset the queue to `WaitQueue::EMPTY` when the head names no live thread, so the object serves again; the option not taken, unlinking inside `Pool::force_release`, needs the endpoint pool, which the thread pool cannot reach.

---

## F08 — issue #80
Title: kernel-types: `any_page_range` carries a panic function for a branch the clamp makes unreachable
Labels: enhancement, part::kernel
Body:
`any_page_range` clamps `count` to `limit - start.number()` (`crates/kernel/types/src/strategies.rs:83-88`), so `PageRange::new(start, count)` at `strategies.rs:89` cannot fail (`crates/kernel/types/src/virt.rs:259-270`, end is at most `limit`). The `unwrap_or_else` chain at `strategies.rs:89-91` and `unreachable_page_range` with its `#[expect(clippy::panic)]` (`strategies.rs:96-103`) exist for that branch.

The crate keeps a `panic!` site and a lint exception for code nothing reaches.

Fix: add `PageRange::EMPTY` beside `PhysFrameRange::EMPTY` (`crates/kernel/types/src/phys.rs:216-220`) and write `PageRange::new(start, count).unwrap_or(PageRange::EMPTY)`, deleting `unreachable_page_range`.
