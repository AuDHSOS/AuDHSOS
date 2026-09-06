# user-programs

The seven programs of the userland: the root task, the name server, the
console driver, the memory server, and the three applications — one that
proves the whole of it works, one that asks the servers the questions the
catalog asks, and one that breaks on purpose so that a fault has somebody
to report it.

Each one is a thin loop around a logic crate that holds what is worth
testing: receive a message, decode it with `user-proto`, ask the policy,
encode the answer, send it back. What is here and nowhere else is the part
that needs a capability or a pointer — mapping a memory object into the
program's own address space to copy bytes into it, reading the archive out
of the boot image, driving the ports of the controller.

This is an adapter crate on the allowlist of the safety policy: a program
that maps memory and then writes into it has to turn an address into a
slice, and that step is `unsafe` wherever it is written.

`server-init` is the root task and is linked at `ROOT_TASK_BASE`, with
every section on a page of its own: the kernel reads it as an ELF and maps
each segment with the permissions its header names, and two segments in one
page would need one set of permissions for both (D-92). The other six are
ordinary executables in the archive, read by `user-loader`.
