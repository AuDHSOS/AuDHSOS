# user-rt

What a user program of `AuDHSOS` works out for itself, with no system call in
it: the typed handles the calls take, the heap over a region of memory the
program owns, the message area of the IPC buffer, the startup message as
named fields, and a line of text in a fixed number of bytes.

The crate makes no system call and knows no instruction, so all of it runs
on the host under test. The trap and the address of the thread's IPC buffer
belong to `user-sys-x86_64`, which depends on this crate and not the other
way round: the heap is arithmetic, and arithmetic does not need a machine.
