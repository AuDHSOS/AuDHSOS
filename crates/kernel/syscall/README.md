# kernel-syscall

What a thread asks the kernel for, and what the kernel checks before it
does anything about it.

One entry point takes the IPC buffer of the calling thread and returns
what the caller should do next. The checks run in the order the interface
documents — number, argument count, handle, object type, rights, arguments,
quota — and the first one that fails decides the error, so an error never
depends on what a later check would have found.

Everything the calls need from outside the logic is one trait,
`Environment`: address spaces, mappings, kernel stacks, frames, and the
debug console. The kernel implements it over its memory bring-up; a test
implements it with a recording double and therefore reaches every error
path on the host.
