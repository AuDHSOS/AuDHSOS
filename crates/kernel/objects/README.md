# kernel-objects

The storage of kernel objects: a fixed-capacity pool whose ids carry a
generation, so that an id of a freed slot is rejected even after the slot
has been reused, and a quota that counts what a process owns. The pool
never allocates; its capacity is a const generic, so that the sizes are
decided where the kernel is assembled and not here.

Beside the pool the crate holds the objects themselves: processes, threads,
memory objects, endpoints, reply objects, notifications, interrupt objects,
and port ranges, with the wait queue an endpoint or a notification keeps its
waiters in. The queue is ordered by priority and then by arrival, and its
links are fields of the thread — the `wait_links` beside the `queue_links`
of the run queues, because a thread is in at most one of the two kinds of
queue at a time (D-74). It lives here rather than in `kernel-ipc` because
its operations need the thread pool; the rendezvous that uses it does not.

The system control capability is the exception that holds nothing: it is
the right to create interrupts, port ranges, and device memory, and there
is nothing to look up behind it, so it has no pool.
