# virtio-queue and driver-virtio-blk audit findings

Repository: AuDHSOS/AuDHSOS. Audit of virtio-queue and driver-virtio-blk at main bc47df5.
Each `## ` section is one issue, filed 2026-09-19 with the number in its heading. `Title:` is the issue title, `Labels:` the labels, everything after `Body:` is the issue body verbatim.

---

## F01 — issue #295
Title: virtio-queue: the used ring is read through a shared slice with plain loads although the device writes it
Labels: bug, part::drivers
Body:
`QueueMemory::used_ring` hands the used ring over as `&[u8]` (`crates/virtio/queue/src/memory.rs:314-315`), and `Queue::next_used` reads the used index and the used element out of that slice with `read_u16` and `read_u32` (`crates/virtio/queue/src/queue.rs:361-362`, `crates/virtio/queue/src/queue.rs:379-381`), which copy the bytes with `first_chunk` and `from_le_bytes` (`crates/virtio/queue/src/memory.rs:341-366`). The device writes those bytes while the slice exists. A `&[u8]` over bytes that change during the borrow is a data race in the Rust memory model, and the compiler may fold a second load of the same bytes into the first when no store and no opaque call stands between them. The one adapter, `crates/user/programs/src/dma.rs:204-206`, reborrows a `&mut [u8]` field over the device-written region, so the adapter cannot make the slice sound either.

`Queue::next_used` returns `Ok(None)` on `crates/virtio/queue/src/queue.rs:363-366` before it calls `barrier`, so a caller that polls `next_used` until it returns `Some` has no fence between two reads of the index. `next_used` is generic over `impl QueueMemory` and is instantiated and inlined in the caller's crate, so loop-invariant code motion may hoist the load of the index out of that loop, and the loop never sees the device's write. The two callers today do not have that shape: `crates/user/programs/src/bin/server_fs.rs:512` makes a system call between polls, and `crates/drivers/virtio-net/src/tx.rs:34` stops on `None`. The trait contract requires neither.

Fix: replace `used_ring(&self) -> &[u8]` in `QueueMemory` by value-returning accessors, `used_u16(&self, at: usize) -> Option<u16>` and `used_u32(&self, at: usize) -> Option<u32>`, that the adapter implements with `read_volatile`, and have `next_used` and `should_notify` read through them; keeping the slice and requiring the adapter to build it per call does not stop the hoist, since the slice is still `Freeze` memory for the compiler.

---

## F02 — issue #298
Title: virtio-queue: a used element naming a descriptor in the middle of a chain is accepted as a head
Labels: bug, part::drivers
Body:
`Queue::next_used` accepts a used id when it is below the size and its free bit is clear (`crates/virtio/queue/src/queue.rs:382-385`). Every descriptor of an outstanding chain has its free bit clear, so a used id that names the second or third descriptor of a chain passes that test. `free_chain` then walks from that descriptor to the end of the chain (`crates/virtio/queue/src/queue.rs:445-462`) and `next_used` returns `Completion { head }` with that id (`crates/virtio/queue/src/queue.rs:395`). The doc of `Completion::head` states the id is "as `Queue::add` returned it" (`crates/virtio/queue/src/queue.rs:51-52`), and the doc of `QueueError::CorruptChain` states the error "says that this crate or its caller has a fault, not that the device misbehaved" (`crates/virtio/queue/src/error.rs:140-143`). The code enforces neither.

A chain of three descriptors A, B, C, published by `add`, and a device that writes id B into the used ring: `next_used` frees B and C, leaves A in use, returns `head: B`, and reports `writable` from B onward. With a request of header, data and status, id C gives `writable = 1`, so the device's true `len` for the data is refused with `UsedLength`. A device that later writes id A makes `free_chain` step from A onto the freed B and return `CorruptChain(A)`. A device that never writes id A leaks A; after `size` such completions the queue answers `QueueFull` to every `add`.

Fix: keep a second `BitSet<WORDS>` of chain heads in `Queue`, set in `publish` and cleared in `free_chain`, and refuse a used id whose head bit is clear with `UnknownDescriptor`; recording the head in the `next` field of the last descriptor would put the check into memory the caller can write.

---

## F03 — issue #301
Title: driver-virtio-blk: the capacity is read after DRIVER_OK, so a generation failure leaves a live device the driver knows nothing about
Labels: bug, part::drivers
Body:
`Blk::initialize` sets `DRIVER_OK` on `crates/drivers/virtio-blk/src/blk.rs:185` and reads the capacity on `crates/drivers/virtio-blk/src/blk.rs:189`. Virtio 1.4 section 3.1.1 puts reading the device configuration space into step 7 and `DRIVER_OK` into step 8. `self.device` is advanced in place by each step, so the comment on `crates/drivers/virtio-blk/src/blk.rs:186-188` holds for `capacity`, `features`, `queue_size` and `notify_offset` and not for the state machine.

A device whose configuration generation changes under every one of the eight reads in `config::stable` (`crates/drivers/virtio-blk/src/config.rs:307-317`) makes `initialize` return `BlkError::Generation(8)` with the queue enabled, the device live (`Blk::device().is_live()` is true), `capacity()` zero, `features()` zero and `notify_offset` zero. `submit` of a flush then succeeds, because `is_read_only()` reads the dropped feature set and `check_data` is not consulted for a flush (`crates/drivers/virtio-blk/src/blk.rs:256-281`), and `notify` writes the queue index to offset zero of the notification structure (`crates/drivers/virtio-blk/src/blk.rs:290-297`), which is the doorbell of queue zero only when its notify offset is zero.

Fix: move the `config::capacity` read to before `configure_queue`, between `negotiate` and the queue setup, and record it with the other fields after `driver_ok`; failing the device with `STATUS_FAILED` on a generation error after `DRIVER_OK` would keep the order but leave the queue enabled on a live device.

---

## F04 — issue #305
Title: virtio-queue: Queue::new claims a nonzero initial used index is refused for the queue's life, but next_used believes it once enough chains are outstanding
Labels: bug, part::drivers
Body:
The doc of `Queue::new` states a used index that starts at something other than zero "is refused for the rest of the queue's life" (`crates/virtio/queue/src/queue.rs:112-115`). `next_used` refuses the index only while `index - last_used` exceeds `in_flight` (`crates/virtio/queue/src/queue.rs:363-373`), and `Queue::new` reads nothing from the used ring (`crates/virtio/queue/src/queue.rs:130-150`).

A used ring whose index reads 3 when `Queue::new` runs, and three chains added by `add`: `pending` is 3, `in_flight` is 3, and `next_used` reads the element in slot 0 (`crates/virtio/queue/src/queue.rs:379-381`), which the device has not written. A slot 0 that holds id 0 while descriptor 0 is in use frees that chain and returns it as complete.

Fix: read the used index in `Queue::new` and return a new `QueueError` variant when it is not zero; correcting the sentence alone leaves the case to the caller's zeroing.

---

## F05 — issue #308
Title: virtio-queue: add does not refuse a chain whose buffer lengths sum past 2^32 bytes
Labels: bug, part::drivers
Body:
Virtio 1.4 section 2.7.5.2 states "Drivers MUST NOT add a descriptor chain longer than 2^32 bytes in total". `Queue::add` checks the count of buffers and their order (`crates/virtio/queue/src/queue.rs:229-237`) and writes each `Buffer::length` into its descriptor (`crates/virtio/queue/src/queue.rs:252-259`) without summing the lengths. `free_chain` saturates the sum of the writable lengths at `u32::MAX` (`crates/virtio/queue/src/queue.rs:454-456`).

A chain of two buffers of `0xFFFF_FFFF` bytes each is published. The device's `len` field cannot represent the chain, and a `len` of `u32::MAX` passes the `UsedLength` check against the saturated sum (`crates/virtio/queue/src/queue.rs:389-394`).

Fix: sum the lengths in `u64` before the first `take_free` and return a new `QueueError` variant when the sum exceeds `u32::MAX`; refusing in `free_chain` would find the fault only after the device has seen the chain.

---

## F06 — issue #311
Title: driver-virtio-blk: the capacity is read once and never refreshed after a configuration change
Labels: enhancement, part::drivers
Body:
`Blk::capacity` reads the field `Blk::initialize` stored (`crates/drivers/virtio-blk/src/blk.rs:189`, `crates/drivers/virtio-blk/src/blk.rs:196-199`), and no method of `Blk` stores a later read. `ISR_CONFIG` is exported (`crates/drivers/virtio-blk/src/blk.rs:86-87`) and no method consumes it. Virtio 1.4 section 5.2.6.2 lets the device change `capacity` during operation, and section 5.2.6.1 has the driver check it on a configuration change notification.

A device that shrinks its capacity from 2048 to 1024 sectors after `initialize`: `check_data` (`crates/drivers/virtio-blk/src/blk.rs:319-340`) keeps comparing against 2048 and `submit` accepts a request for sector 1500, which section 5.2.6.1 forbids the driver to submit.

Fix: add `Blk::refresh_capacity(&mut self, registers: &impl Registers) -> Result<u64, BlkError>` that calls `config::capacity` and stores the result, for the caller to run when `interrupt_status` carries `ISR_CONFIG`.

---

## F07 — issue #313
Title: virtio-queue: the barrier doc counts four call sites where the code has three
Labels: enhancement, part::drivers
Body:
The doc of `QueueMemory::barrier` states "Three of the four places this is called are required by name" and names step 4, step 6 and "the fourth" between the used index and the element (`crates/virtio/queue/src/memory.rs:320-330`). `barrier` is called at `crates/virtio/queue/src/queue.rs:301`, `crates/virtio/queue/src/queue.rs:378` and `crates/virtio/queue/src/queue.rs:410`.

A reader looking for the third named call site finds none.

Fix: write "Two of the three places this is called are required by name" and "the third".

---

## F08 — issue #315
Title: virtio-queue: a test message places the reaping barrier before the used index
Labels: enhancement, part::drivers
Body:
The assertion message in `publishing_and_reaping_ask_for_a_barrier` reads "before the used index is read" (`crates/virtio/queue/src/tests/queue.rs:491`). The barrier in `next_used` is after the index is read and before the element (`crates/virtio/queue/src/queue.rs:374-378`), and the comment there states the barrier does not belong before the index.

A failing assertion prints an order the code does not have.

Fix: change the message to "between the used index and the element".
