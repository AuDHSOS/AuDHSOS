# kernel-syscall audit findings

Repository: AuDHSOS/AuDHSOS. Audit of kernel-syscall at main bc47df5.
Each `## ` section is one issue, filed 2026-09-19 with the number in its heading. `Title:` is the issue title, `Labels:` the labels, everything after `Body:` is the issue body verbatim.

---

## F01 — issue #191
Title: kernel-syscall: memory_protect grants write and execute without checking the rights of the backing object
Labels: bug, part::kernel
Body:
`memory::map` refuses a writable mapping of a memory object whose handle lacks `WRITE` and an executable one whose handle lacks `EXECUTE` (`crates/kernel/syscall/src/calls/memory.rs:86-92`). `memory::protect` takes the permission bits of argument 3 and writes them into the page tables and the region table without any check against the backing object (`crates/kernel/syscall/src/calls/memory.rs:271-295`). The region records the backing object id and the permissions, not the rights of the handle the mapping was made through (`crates/kernel/mm/src/address_space.rs:91-102`), so the check cannot be done after `memory_map` either.

A process holding a memory object handle with `READ | MAP` maps it read-only, then calls `memory_protect(self, address, length, 0b11)` and gets a writable, executable mapping of memory it was granted read-only. Every program the root task starts holds its own process handle with `MAP` (`crates/user/programs/src/bin/server_init.rs:1242-1270`), so any client of a server that shares an object read-only can write into it, and any process can make data pages executable, against 2.11 "Rights only decrease along duplication and transfer" (`docs/02-architecture.md:625`).

Fix: record in `Region` the rights the mapping handle carried and refuse in `protect` a `write` without `WRITE` and an `execute` without `EXECUTE`; the option not taken, dropping `memory_protect` and requiring unmap plus map, costs the page-table churn D-104 avoids.

---

## F02 — issue #192
Title: kernel-syscall: a queued sender whose copy fails is requeued at the front and wedges the endpoint for every receiver
Labels: bug, part::kernel
Body:
`ipc::send` checks the message counts and the label before queuing and does not resolve the handle words (`crates/kernel/syscall/src/calls/ipc.rs:131-139`, `crates/kernel/syscall/src/calls/ipc.rs:451-457`). The handles are resolved at rendezvous time in `transfer` (`crates/kernel/ipc/src/transfer.rs:77-84`). When the receiver meets a queued sender and the copy fails, `recv` puts the sender back with `undo_meeting` and returns the error to the receiver (`crates/kernel/syscall/src/calls/ipc.rs:281-289`); `undo_meeting` puts the peer back at the front of its queue (`crates/kernel/ipc/src/endpoint.rs:299-337`).

A client writes handle count 1 and a handle word that names nothing, or a handle without `TRANSFER` such as a reply handle, and calls `ipc_send` on a server endpoint while the server is not receiving. Every later `ipc_recv` and `ipc_try_recv` on that endpoint meets the same sender, fails with `InvalidHandle` or `AccessDenied`, requeues it at the front, and returns the error to the server. One client with `SEND` stops a server for good, and nothing tells the client.

Fix: on a copy that fails in `recv`, wake the queued sender with `Status::failed(error)` through `write_result` and take the next sender instead of requeuing; the option not taken, resolving the handles in `check_header` at send time, leaves the same window for a handle closed while the sender is queued.

---

## F03 — issue #193
Title: kernel-syscall: thread_create accepts one memory object as the IPC buffer of two threads, which lets the kernel hold two mutable references to one frame
Labels: bug, part::kernel
Body:
`supplied_buffer` checks that the object is `Ram` and one frame and nothing else (`crates/kernel/syscall/src/calls/thread.rs:214-227`); a creator can name the same object for two threads, and each retains it once (`crates/kernel/syscall/src/calls/thread.rs:245`). `deliver` and `recv` refuse a rendezvous between two threads on one frame with a comment that says the state is unreachable (`crates/kernel/syscall/src/calls/ipc.rs:169-174`, `crates/kernel/syscall/src/calls/ipc.rs:260-263`). `answer` has no such check (`crates/kernel/syscall/src/calls/ipc.rs:417-424`), and neither has `write_result` (`crates/kernel/syscall/src/calls/ipc.rs:508-516`). The gate holds `&mut [u8; SIZE]` over the caller's frame for the whole call under a `SAFETY` comment that names it the only writer (`crates/kernel/bin/src/main.rs:136-142`), and `with_buffer` builds a second `&mut [u8; SIZE]` over whatever frame it is given (`crates/kernel/core/src/syscall.rs:282-292`).

A process creates threads T1 and T2 with the same one-page object and T3 with a buffer of its own. T1 calls `ipc_call` on an endpoint, T3 receives and gets the reply handle, T2 calls `ipc_reply` with it: `answer` passes T2's buffer as `from` and opens T1's frame, the same frame, as `to`, so `transfer` runs with a shared and a mutable reference to one page, which is undefined behavior. `thread_suspend` of T1 by T2 reaches `write_result` on the same frame the gate holds, and so does `lifetime::release` waking T1.

Fix: refuse in `supplied_buffer` an object whose frame is the `ipc_buffer` of a live thread, a scan of the thread pool in O(NT); the option not taken, comparing frames in `answer` and `write_result` too, keeps a state the gate's `SAFETY` comment excludes.

---

## F04 — issue #194
Title: kernel-syscall: memory_split of a mapped object leaves its tail frames mapped while the tail's reference count reads one
Labels: bug, part::kernel
Body:
`memory::split` checks the offset and the pool and moves the tail frames into a new object without checking whether anything maps the source (`crates/kernel/syscall/src/calls/memory.rs:321-369`). A region records the backing object id and an offset (`crates/kernel/mm/src/address_space.rs:91-102`), so a region that covered frames beyond the new head still names the head object and still maps the frames the tail now owns. `memory_merge` and the memory server rely on a count of one as proof that nothing maps the object (`crates/kernel/syscall/src/calls/memory.rs:437-443`, `docs/02-architecture.md:519`).

A client maps a two-page object at some address, calls `memory_split(handle, 4096)`, transfers the tail handle to the memory server and closes its own. `memory_references` on the tail answers 1 while the client still reads and writes the tail's frame through the mapping of the head. The memory server recycles the tail to another client, and the two clients share a frame.

Fix: refuse `memory_split` with `Busy` unless `references(id)` is one, as `memory_merge` does; the option not taken, splitting the regions that span the offset, needs a region per part and a retain per region.

---

## F05 — issue #195
Title: kernel-syscall: the kernel-object quota is charged at creation and never refunded when an object is destroyed
Labels: bug, part::kernel
Body:
`endpoint_create`, `notification_create`, `memory_split`, `interrupt_create`, `interrupt_create_msi`, `ioport_create`, and `memory_create_device` charge one object against the caller's `kernel_object_quota` (`crates/kernel/syscall/src/calls/mod.rs:149-166`, `crates/kernel/syscall/src/calls/ipc.rs:63`, `crates/kernel/syscall/src/calls/memory.rs:345`). `lifetime::release` drops the reference and destroys the object without touching a quota (`crates/kernel/syscall/src/lifetime.rs:26-50`), and `Objects::destroy` frees the pool slot only (`crates/kernel/objects/src/store.rs:296-324`). The only refunds are a thread's exit (`crates/kernel/syscall/src/calls/thread.rs:527`), `memory_merge` (`crates/kernel/syscall/src/calls/memory.rs:451`), and the failure paths of the creating calls. D-90 states that a destroyed object goes back to the quota (`docs/09-decisions.md:100`).

A program with an object quota of 64, which is what the root task grants most programs (`crates/user/programs/src/bin/server_init.rs:161-162`), creates a notification and closes its handle 64 times; the 65th `notification_create` fails with `QuotaExceeded` and every later creating call fails the same way for the life of the process.

Fix: record the process that was charged in each pool object and refund it in `lifetime::release` when `destroy` reports the object gone; the option not taken, refunding in `handle_close`, refunds a handle that was not the last reference.

---

## F06 — issue #196
Title: kernel-syscall: process_create takes the child's quotas out of the creator and process_kill never gives them back
Labels: bug, part::kernel
Body:
`process::create` charges the creator's `quota` by `frames` and its `kernel_object_quota` by `objects + 1` (`crates/kernel/syscall/src/calls/process.rs:165-178`). `process::kill` releases the child's threads, handles, address space, and slot and refunds nothing to anybody (`crates/kernel/syscall/src/calls/process.rs:340-383`). `Process` records no creator (`crates/kernel/objects/src/object.rs:226-246`), so no later call can refund either.

A server with an object quota of 64 that starts a helper process with `objects = 8` and kills it can do so seven times; the eighth `process_create` fails with `QuotaExceeded` although no helper is alive. The root task escapes because it holds `u32::MAX` of both (`crates/kernel/core/src/root.rs:120-124`).

Fix: record the creator in `Process` and refund `frames`, `objects`, and the one object of the process itself in `kill` and when the last reference to an ended process goes; the option not taken, refunding in `process_create` only on failure, is what the code does today.

---

## F07 — issue #197
Title: kernel-syscall: destroying a bound line interrupt object leaves the line unmasked with no object to mask it
Labels: bug, part::kernel
Body:
`interrupt_bind` unmasks the line (`crates/kernel/syscall/src/calls/device.rs:199-202`). `lifetime::release` gives back a message vector when the last reference to a message interrupt goes and does nothing for a line (`crates/kernel/syscall/src/lifetime.rs:36-49`, `crates/kernel/syscall/src/lifetime.rs:52-74`). The interrupt path masks a line only when an object names its vector (`crates/kernel/bin/src/main.rs:282-300`, `crates/kernel/ipc/src/interrupt.rs:29-38`).

A driver binds its interrupt and its process is killed, or it closes the last handle, while the device asserts a level-triggered line. Every assertion enters `on_interrupt`, finds no object, acknowledges without masking, and returns to an asserted line; the machine spends its time in the interrupt handler.

Fix: in `lifetime::release`, before `destroy`, read the line of an interrupt object whose count is one and call `mask_interrupt` on it; the option not taken, masking in `forward` when no object is found, masks lines the kernel routed for itself.

---

## F08 — issue #198
Title: kernel-syscall: a line stays routed after its interrupt object is gone or its creation fails, so the line can never be created again
Labels: bug, part::kernel
Body:
`interrupt_create` routes the line at the controller and then allocates the object and installs the handle; the two failure paths after the route refund the quota and do not unroute (`crates/kernel/syscall/src/calls/device.rs:72-98`). `lifetime::release` keeps the routing of a line interrupt on purpose (`crates/kernel/syscall/src/lifetime.rs:52-54`). `Environment` has no operation that unroutes a line (`crates/kernel/syscall/src/environment.rs:213-225`), and the controller refuses a second route of a routed line with `AlreadyExists` (`crates/kernel/hal-x86_64/src/apic.rs:470-480`, `crates/kernel/syscall/src/environment.rs:213-219`).

The root task calls `interrupt_create(line)` while the interrupt pool is full: `route_interrupt` succeeds, `allocate` fails with `PoolExhausted`, and every later `interrupt_create(line)` fails with `AlreadyExists` because `interrupt_of_line` finds no object and the controller finds the entry set. The same happens after a driver's interrupt object is destroyed.

Fix: let `route_interrupt` accept a line already routed to the same vector, since the vector of a line is fixed by the plan (`crates/kernel/syscall/src/calls/device.rs:41-43`); the option not taken, an `unroute_interrupt` on `Environment` called from both places, adds an operation to every implementation.

---

## F09 — issue #199
Title: kernel-syscall: a thread_create that fails in a process without threads marks the process ended and signals its watchers
Labels: bug, part::kernel
Body:
Every failure of `thread::create` after `add_thread` goes through `forget_thread`, which removes the thread and calls `watch::thread_left` (`crates/kernel/syscall/src/calls/thread.rs:149-159`, `crates/kernel/syscall/src/calls/thread.rs:334-352`). `thread_left` treats a process with no thread as ended (`crates/kernel/syscall/src/watch.rs:45-53`), and `ended` sets the flag and signals every watcher (`crates/kernel/syscall/src/watch.rs:70-85`).

The root task creates process P, a server calls `process_watch(P, n, bit)`, and the root task's first `thread_create` for P fails at `map_buffer` with `OutOfKernelMemory` or at `install` with `OutOfHandles`. The server's bit is signalled although P never ran. The root task then retries and succeeds; when P's last thread later exits, `ended` finds the flag set and tells nobody (`crates/kernel/syscall/src/watch.rs:77-79`), and a `process_watch` on the running P signals at once (`crates/kernel/syscall/src/calls/process.rs:76-83`).

Fix: make `forget_thread` remove the thread without calling `thread_left`, since a process that never held a started thread has not ended; the option not taken, refusing `thread_create` on an ended process, does not cover this path.

---

## F10 — issue #200
Title: kernel-syscall: memory_protect changes the page tables before the region table can refuse, so a failed call leaves the two apart
Labels: bug, part::kernel
Body:
`memory::protect` writes the new permissions page by page into the page tables (`crates/kernel/syscall/src/calls/memory.rs:279-284`) and only then calls `regions.protect` (`crates/kernel/syscall/src/calls/memory.rs:292-295`). `RegionTable::protect` refuses a range that is not inside one region with `NotMapped` and a split the table has no room for with `QuotaExceeded` (`crates/kernel/mm/src/address_space.rs:552-571`). The page loop stops at the first page without a translation with `NotMapped` and undoes nothing (`crates/kernel/syscall/src/calls/memory.rs:281-284`). The module invariant says a mapping is recorded in the region table exactly as it exists in the page tables (`crates/kernel/syscall/src/calls/memory.rs:6-9`), and the dispatcher invariant says a call that fails changes nothing (`crates/kernel/syscall/src/dispatch.rs:11`).

A process with adjacent regions A over pages 0-1 and B over pages 2-3 calls `memory_protect` over pages 0-3: all four pages become read-only in the page tables, `regions.protect` answers `NotMapped`, and the call fails with the region table still saying read-write. A range that crosses a gap changes the pages before the gap and fails the same way.

Fix: call `regions.protect` first and change the page tables only after it succeeded, unwinding the pages already changed if a page write fails; the option not taken, unwinding after a region failure, rewrites every page twice on the error path.

---

## F11 — issue #201
Title: kernel-syscall: ipc_reply_recv reports failure after the answer went out and drops the switch the answer asked for
Labels: bug, part::kernel
Body:
`reply_recv` answers the caller, writes its wake-up, consumes the reply handle, and then runs `recv` (`crates/kernel/syscall/src/calls/ipc.rs:395-400`). A `recv` that fails returns `Err` from the call, and the dispatcher writes the error status and answers `Outcome::NOTHING` (`crates/kernel/syscall/src/dispatch.rs:347-350`), so `answered.reschedule` and `switched` are lost (`crates/kernel/syscall/src/calls/ipc.rs:401-403`).

A server whose handle table is full calls `ipc_reply_recv`: the client is woken with its answer, the reply handle is gone, `open_reply` for the next caller fails with `QuotaExceeded`, and the server reads a failed status with no way to tell that the reply was delivered; a retry with the same handle fails with `InvalidHandle`. A woken client of higher priority is not switched to until the next tick.

Fix: return the outcome beside the error from `handle` so that `dispatch` writes the error status and still returns the switch, and state in the call's documentation that the reply is delivered when the status names a receive error; the option not taken, reserving the reply slot and handle before the answer, changes the check order of 2.8.

---

## F12 — issue #202
Title: kernel-syscall: memory_map maps an object whose handle lacks READ, and every mapping is readable
Labels: bug, part::kernel
Body:
`memory::map` resolves the object handle with `MAP` and checks `WRITE` and `EXECUTE` against the permission bits; it never checks `READ` (`crates/kernel/syscall/src/calls/memory.rs:79-92`), and a mapping always allows reading (`crates/kernel/syscall/src/calls/memory.rs:22-24`). `READ` is defined as the right to read the contents of a memory object (`crates/abi/src/rights.rs:40`, `docs/02-architecture.md:73`).

A process that holds a memory object handle with `MAP` alone maps it and reads every byte. A holder that duplicates a handle without `READ` to withhold reading withholds nothing.

Fix: require `READ` beside `MAP` in `memory::map` and in `supplied_buffer`; the option not taken, dropping `READ` from the mask of `MemoryObject`, changes the ABI.

---

## F13 — issue #203
Title: kernel-syscall: memory_map checks only the first page against the region table and answers AlreadyMapped for an overlap further on
Labels: bug, part::kernel
Body:
`memory::map` refuses with `AddressInUse` when the first page of the request is in a region (`crates/kernel/syscall/src/calls/memory.rs:107-110`). A region that starts inside the request is found by `Environment::map`, which answers `AlreadyMapped` (`crates/kernel/syscall/src/environment.rs:45-52`), after which the pages already mapped are unwound (`crates/kernel/syscall/src/calls/memory.rs:121-127`).

A process with a region at page 3 calls `memory_map` over pages 0-4: pages 0-2 are mapped and unmapped again, and the call fails with `AlreadyMapped` where the same overlap at page 0 fails with `AddressInUse`; the documented error for the case is `AddressInUse` (`crates/kernel/syscall/src/calls/memory.rs:67-68`).

Fix: replace the `find` on the first page with an overlap check of the whole budget range against the region table before the loop; the option not taken, mapping `AlreadyMapped` to `AddressInUse` in the loop, keeps the map-and-unwind cost.

---

## F14 — issue #204
Title: kernel-syscall: 2.4.2 says page-table frames count against the kernel-object quota, and memory_map charges nothing
Labels: bug, part::kernel
Body:
`docs/02-architecture.md:181-183` states that page-table frames come from the kernel reserve and count against the process's kernel-object quota. `memory::map` calls `Environment::map` per page and charges no quota (`crates/kernel/syscall/src/calls/memory.rs:112-129`); the kernel's `map` allocates page tables from the reserve without a charge (`crates/kernel/core/src/syscall.rs:196-207`).

A process with an object quota of 8 maps 64 one-page regions at addresses 512 GiB apart and holds 192 reserve frames in page tables, 24 times its quota; the reserve is what every `thread_create` and `process_create` on the machine needs.

Fix: charge the process one object per page-table frame `Environment::map` allocates and refund it when `unmap` frees one, which needs the count of allocated tables in the result of `map`; the option not taken, striking the sentence from 2.4.2, leaves the reserve uncounted.

---

## F15 — issue #205
Title: kernel-syscall: 2.8 says thread_create takes an IPC buffer as memory object and offset, and the call takes a handle alone
Labels: bug, part::kernel
Body:
`docs/02-architecture.md:495` lists the arguments of `thread_create` as entry, stack pointer, IPC buffer as memory object and offset, priority, maximum priority. The table has six arguments (`crates/abi/src/syscall.rs:129`), the sixth is a memory object handle of one page or zero and no offset exists (`crates/kernel/syscall/src/calls/mod.rs:20`, `crates/kernel/syscall/src/calls/thread.rs:85-90`, `crates/kernel/syscall/src/calls/thread.rs:214-227`).

A reader of 2.8 passes an offset word and gets `ArgumentCount` from `decode` (`crates/kernel/syscall/src/dispatch.rs:262-265`).

Fix: rewrite the row as `entry, stack pointer, priority, maximum priority, memory object of one page for the IPC buffer or zero`.

---

## F16 — issue #206
Title: kernel-syscall: 2.8 says debug_log exists only in builds with the debug-uart feature, and the call is in every build
Labels: bug, part::kernel
Body:
`docs/02-architecture.md:520` states that `debug_log` exists only in builds with the `debug-uart` feature. The call is in the table unconditionally (`crates/abi/src/syscall.rs:165`), `debug::log` answers the byte count in every build (`crates/kernel/syscall/src/calls/debug.rs:28-53`), and `Environment::log` drops the bytes when there is no console (`crates/kernel/syscall/src/environment.rs:118-120`); the kernel stops writing once the root task hands the port to the console driver (`crates/kernel/bin/src/task.rs:284-286`).

A program that probes for the feature by calling `debug_log` and expecting `UnknownSyscall` sees `OK` on every build.

Fix: rewrite the row as `writes the message region to the debug console; a build without one drops the bytes and answers the count`.

---

## F17 — issue #207
Title: kernel-syscall: process_kill of the caller's own process releases the active page-table root before the switch
Labels: enhancement, part::kernel
Body:
`process::kill` calls `destroy_address_space` on the target's root (`crates/kernel/syscall/src/calls/process.rs:363`), and the kernel's implementation releases the root frame to the reserve (`crates/kernel/mm/src/kernel_half.rs:169-194`, `crates/kernel/mm/src/kernel_half.rs:192`). When the target is the caller's process, the processor keeps that frame in `CR3` through the rest of the call, the write of the result, and the reaper the gate runs before the switch (`crates/kernel/bin/src/task.rs:302-315`).

Nothing on that path allocates a frame today, so the freed root is not overwritten before the switch; an allocation added to `watch::ended`, to `reap`, or to the result write would zero the root of the running address space.

Fix: defer `destroy_address_space` of the caller's own root to the reaper, which runs after the switch (`crates/kernel/syscall/src/reaper.rs:33-37`), by recording the root in the process until its last thread is cleared; the option not taken, switching to the kernel root before the destroy, needs an operation the `Environment` trait does not have.

---

## F18 — issue #208
Title: kernel-syscall: process_kill copies the whole Process onto the kernel stack to iterate its threads
Labels: enhancement, part::kernel
Body:
`process::kill` copies the `Process` out of the pool (`crates/kernel/syscall/src/calls/process.rs:346`) and reads three fields of it: `threads()`, `root`, and `fault_handler` (`crates/kernel/syscall/src/calls/process.rs:353-370`). A `Process` is about four kilobytes of region table and thread slots (`crates/abi/src/layout.rs:77-80`), on a kernel stack of eight pages sized by the deepest path D-73 measured (`crates/abi/src/layout.rs:73-81`).

The copy costs four kilobytes of the thirty-two the stack has, on a path that then runs `close_every_handle`, `lifetime::release`, and `watch::ended` below it.

Fix: copy the thread array, the root, and the fault handler into locals instead of the whole `Process`.

---

## F19 — issue #209
Title: kernel-syscall: the Thread::new error in thread_create is unreachable and would leak the stack, the buffer, and the quota
Labels: enhancement, part::kernel
Body:
`thread::create` refuses a `max_priority` at or above `PRIORITY_COUNT` and a `priority` above it before any state changes (`crates/kernel/syscall/src/calls/thread.rs:91-93`). `Thread::new` checks the same condition and nothing else (`crates/kernel/objects/src/object.rs:428-437`). The call uses `?` on it after the quota was charged, the kernel stack taken, and the buffer taken (`crates/kernel/syscall/src/calls/thread.rs:117`), and that return runs none of the undo steps every other failure runs (`crates/kernel/syscall/src/calls/thread.rs:102-116`).

No input reaches the error today; a change to `Thread::new` that adds a check makes `thread_create` leak a kernel stack slot, a reserve frame or an object reference, and one object of the quota per failure.

Fix: replace the `?` with a match that runs the same undo steps as the `allocate` failure below it; the option not taken, removing the check from `Thread::new`, makes the constructor accept what it documents as refused.

---

## F20 — issue #210
Title: kernel-syscall: debug_log carries an error path that no input reaches
Labels: enhancement, part::kernel
Body:
`trailing_bytes` answers a value from one to eight (`crates/kernel/syscall/src/calls/debug.rs:57-62`), and `bytes` is an array of eight (`crates/kernel/syscall/src/calls/debug.rs:39`), so `bytes.get(..take)` is always `Some` and the `InvalidArgument` return is dead (`crates/kernel/syscall/src/calls/debug.rs:46-48`). The doc comment lists `InvalidArgument` for the header only (`crates/kernel/syscall/src/calls/debug.rs:24-27`).

A reader of the function looks for the input that produces the second `InvalidArgument` and finds none.

Fix: take the slice with `bytes.get(..take).unwrap_or(bytes.as_slice())` or index the array by a `take` typed as `usize` bounded by `WORD`, and drop the error return.
