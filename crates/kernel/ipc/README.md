# kernel-ipc

The rendezvous: which thread meets which, what a message carries across,
and what a destroyed object owes the threads that waited on it.

An endpoint holds two queues, one of threads that want to send and one of
threads that want to receive, both ordered by priority and then by arrival.
Whichever side arrives second completes the meeting, so every operation here
is two steps: one that finds the peer or queues the caller, and one that
says what became of both once the message has moved. The step in between is
the copy, which needs two IPC buffers and therefore belongs to the system
call layer; nothing here reaches a frame or an address space.

Every operation returns an `Outcome`: whether the caller blocks, what its
status and return words are when it does not, which thread became ready and
what goes into its buffer, and whether the caller should switch before it
returns to user mode. A destroyed object returns `Waiters` instead, which
hands out the threads it left one at a time, because there can be as many of
them as there are threads and each of them needs its own buffer written.
