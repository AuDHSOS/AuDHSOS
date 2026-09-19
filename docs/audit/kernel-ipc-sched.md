# kernel-ipc and kernel-sched audit findings

Repository: AuDHSOS/AuDHSOS. Audit of kernel-ipc, kernel-sched at main bc47df5.
Each `## ` section is one issue, filed 2026-09-19 with the number in its heading. `Title:` is the issue title, `Labels:` the labels, everything after `Body:` is the issue body verbatim.

---

## F01 — issue #52
Title: kernel-ipc: a dropped reply object wakes a caller that stopped waiting for it
Labels: bug, part::kernel
Body:
`destroy_reply` at `crates/kernel/ipc/src/destroy.rs:34-41` hands out `reply.caller` whenever `reply.consumed` is false. `Waiters::wake_next` at `crates/kernel/ipc/src/outcome.rs:191-196` then overwrites the caller's `wait` with `Wait::Nothing` and wakes the caller when its state is any blocked state. Neither function compares the caller's `wait` record with the reply object. `cancel` at `crates/kernel/ipc/src/cancel.rs:55-63` clears the caller's record on a suspend and leaves the reply object with `consumed == false` and `caller` set. `reply_caller` at `crates/kernel/ipc/src/endpoint.rs:397-402` makes that comparison; the destroy path does not.

Trigger: thread A does `ipc_call` on endpoint E, the server takes it, and A waits in `BlockedReply` with `Wait::Reply { reply: r }` (`crates/kernel/ipc/src/endpoint.rs:262-263`). A's creator calls `thread_suspend(A)` (`crates/kernel/syscall/src/calls/thread.rs:450-454`), which runs `cancel` and suspends A. The creator calls `thread_resume(A)`. A runs and calls `ipc_recv(F)`, entering F's receivers queue in `BlockedRecv` with `Wait::Endpoint` (`crates/kernel/ipc/src/endpoint.rs:217-231`). The server closes its handle to r; `crates/kernel/syscall/src/lifetime.rs:45-47` calls `destroyed` and `wake_next`. `wake_next` clears A's record and `wake` (`crates/kernel/ipc/src/outcome.rs:228-234`) moves A to `Ready`, so A returns from `ipc_recv` with `ReplyDropped` while its `wait_links` still chain it into F's receivers queue. A later `ipc_send(F)` dequeues A (`crates/kernel/ipc/src/endpoint.rs:146-149`) and the system call layer copies the message into A's IPC buffer while A is ready or running. If A was the only receiver on F, its links are `UNLINKED` and a following `ipc_recv(G)` by A enqueues it into G as well (`crates/kernel/objects/src/wait_queue.rs:88`), after which an `unlink` from F rewrites G's chain: two wait queues share one thread's links.

Fix: in `cancel`, the `Wait::Reply { reply }` arm sets `consumed = true` on the reply object (`objects.replies.with(reply, |held| held.consumed = true)`), so `destroy_reply` answers `Waiters::one(None, ..)` and `reply_caller` keeps answering `InvalidState`; the option not taken is carrying the `ReplyId` in `Waiters` and comparing it with the caller's record in `wake_next`, which touches three types instead of one arm.

---

## F02 — issue #53
Title: kernel-ipc: transfer copies a queued sender's message without checking the reserved label
Labels: bug, part::kernel
Body:
`transfer` at `crates/kernel/ipc/src/transfer.rs:71-72` reads the header through `Buffer::message`, which checks the two counts and not the label. The module documentation at `crates/kernel/ipc/src/transfer.rs:19-23` places the reserved-label check at the entry of `ipc_send` and `ipc_call`; `check_header` at `crates/kernel/syscall/src/calls/ipc.rs:451-457` runs there (`crates/kernel/syscall/src/calls/ipc.rs:134` and `:416`) against the sender's buffer at that moment. For a sender that found no receiver and queued, the copy happens later in `recv` (`crates/kernel/syscall/src/calls/ipc.rs:277-280`), reading the sender's IPC buffer frame again. That frame is mapped in the sender's process at `Thread::ipc_address` (`crates/kernel/objects/src/object.rs:381-384`), where every thread of the process can write it.

Trigger: a process with threads T1 and T2 holds a send capability to the endpoint a fault handler receives on (`docs/02-architecture.md:413`, section 2.6.4). T1 calls `ipc_call` with an ordinary label and no receiver waits, so T1 queues. T2 writes a label in the kernel range and three words into T1's IPC buffer. The handler's `ipc_recv` copies the rewritten message; `Message::is_kernel_label` is true for it, so the handler cannot tell it from a fault message the kernel built. The same holds for `ipc_send`.

Fix: `transfer` takes a `kernel_message: bool` and returns `Error::InvalidArgument` when it is false and `message.is_kernel_label()` holds, with the fault path (`crates/kernel/syscall/src/fault.rs`) the only caller passing true; the option not taken is repeating `check_header` in `recv` before the copy, which leaves the invariant outside the one function that moves a message.

---

## F03 — issue #54
Title: kernel-sched: a thread preempted by a higher priority goes to the tail with a fresh slice
Labels: enhancement, part::kernel
Body:
`pick_next` at `crates/kernel/sched/src/scheduler.rs:316-325` applies `Event::Preempt` to the running thread and calls `enqueue`, which appends at the tail (`crates/kernel/sched/src/scheduler.rs:236-255`); `crates/kernel/sched/src/scheduler.rs:337-338` gives the picked thread `DEFAULT_TIME_SLICE_TICKS`. The same path serves both cases `Event::Preempt` names at `crates/kernel/sched/src/transition.rs:20-22`: slice expiry and loss of the processor to a higher priority. The remaining ticks of the outgoing thread are discarded either way.

Consequence: with threads A, B, C at one priority and a higher-priority thread H that wakes on every tick (a `notification_wait_until` deadline or an interrupt), each wake of H rotates the queue, so A, B, C each run one tick per turn. The effective slice equals H's period and not the ten ticks of `DEFAULT_TIME_SLICE_TICKS` (`crates/abi/src/layout.rs:167`); `docs/02-architecture.md:314-315` describes round robin with a slice measured in timer ticks.

Fix: split `Event::Preempt` into slice expiry, which keeps the tail placement, and displacement by a higher priority, which puts the thread at the head of its queue and keeps `time_slice`; the option not taken is keeping tail placement and only preserving `time_slice`, which still hands the processor to a peer on every displacement.

---

## F04 — issue #55
Title: kernel-sched: the deadline list insert walks the list, O(n) per wait
Labels: enhancement, part::kernel
Body:
`insert_deadline` at `crates/kernel/sched/src/scheduler.rs:447-498` calls `place_for` at `crates/kernel/sched/src/scheduler.rs:502-519`, which walks the list from the tail until it finds an entry whose deadline is at or before the new one. The walk is O(n) in the number of waiting threads, bounded by `waiting`. `expired` at `crates/kernel/sched/src/scheduler.rs:382-419` is O(1) per removal.

Consequence: a process whose k threads call `notification_wait_until` with decreasing deadlines makes every insert walk the whole list: k inserts cost O(k^2), with k bounded by the thread pool. `docs/10-implementation-plan.md:2954-2957` records the tail walk as the design and names a deadline later than every other as the common case.

Fix: a fixed-capacity binary heap of `(deadline, ThreadId)` with the pool size as capacity gives O(log n) insert and O(log n) pop and needs no allocation; the option not taken is keeping the list, which is correct and O(1) in the common case.

---

## F05 — issue #56
Title: kernel-sched: the transition lookup scans the table, O(rows) per state change
Labels: enhancement, part::kernel
Body:
`next` at `crates/kernel/sched/src/transition.rs:204-210` finds a transition by a linear search over the 37 rows of `TRANSITIONS` (`crates/kernel/sched/src/transition.rs:100-195`). `apply` at `crates/kernel/sched/src/scheduler.rs:565-566` and `pick_next` at `crates/kernel/sched/src/scheduler.rs:337` call it on every block, wake, exit, suspend, resume, preempt, and schedule.

Consequence: every state change costs up to 37 comparisons of two enum pairs, on the path of every system call that blocks or wakes a thread and of every timer-driven switch.

Fix: a `const` two-dimensional table `[[Option<ThreadState>; EVENTS]; STATES]` built from `TRANSITIONS` in a `const` block, indexed by the discriminants, making `next` O(1); the option not taken is ordering the rows by state and stopping early, which stays O(rows).

---

## F06 — issue #57
Title: kernel-ipc: the deliver_word comment names a case cancel already covers
Labels: enhancement, part::kernel
Body:
The comment at `crates/kernel/ipc/src/notify.rs:59-61` names "a `thread_resume` of one that was suspended out of its wait" as a waiter that stopped waiting without the notification knowing. `thread_suspend` at `crates/kernel/syscall/src/calls/thread.rs:450-451` calls `cancel` first, and the `Wait::Notification` arm at `crates/kernel/ipc/src/cancel.rs:48-54` sets `waiter = None`, so `deliver_word` at `crates/kernel/ipc/src/notify.rs:53-55` returns before the guard at `:62-64` for that case.

Consequence: a reader looking for the state the guard at `crates/kernel/ipc/src/notify.rs:62-64` protects against is sent to a sequence the code does not produce; the guard's remaining purpose is a stale `waiter` from a path that skips `cancel`.

Fix: rewrite the comment to name the guard's actual role, a stale `waiter` left by a path that did not run `cancel`, and cite `cancel`; the option not taken is deleting the guard, which removes the last defense against a stale record.

---

## F07 — issue #58
Title: kernel-ipc: received takes a receiver argument it discards
Labels: enhancement, part::kernel
Body:
`received` at `crates/kernel/ipc/src/endpoint.rs:277-284` declares `receiver: ThreadId` and discards it with `let _ = receiver;`. The one caller at `crates/kernel/syscall/src/calls/ipc.rs:296` passes `caller`.

Consequence: the signature claims the function acts on the receiver; every call site pays a lookup for an argument that changes nothing.

Fix: remove the parameter from `received` and from the call site; the option not taken is keeping it for symmetry with `sent`, which reads its `receiver`.
