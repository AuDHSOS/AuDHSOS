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

What a thread may do next is a table, not a series of conditions: every
legal transition is one row of `transition::TRANSITIONS`, and every pair the
table does not name is an error rather than a panic.
