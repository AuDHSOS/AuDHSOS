# kernel-sched

The scheduler: which thread runs next, and what a state change means.

Thirty-two priorities, each with a run queue of threads in first-in
first-out order, and a bitmap that names the priorities that hold anyone, so
picking the next thread is one instruction on the bitmap and one queue head.
Higher priority always wins; equal priorities share the processor round
robin with a time slice measured in timer ticks.

The queues are intrusive: a thread carries the links of the queue it is in,
so enqueueing costs no memory and no allocation. The scheduler therefore
receives the thread pool with every call.

`transition::TRANSITIONS` defines legal state changes. A const lookup table
built from those rows answers each state/event pair in O(1). Missing pairs
return `InvalidState`.

Each processor keeps a deadline min-heap with `kernel_objects::config::THREADS`
entries and no allocation. Insertion, expiry, and cancellation cost O(log n);
checking an unexpired minimum costs O(1). Threads record their heap positions
for cancellation. Equal deadlines expire in FIFO order. Stale entries are
removed individually without discarding other deadlines.
