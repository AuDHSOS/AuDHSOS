# user-sys-x86_64

The two things a user program cannot write in safe Rust: the entry point
the kernel jumps to, and the instruction that makes a system call.

A thread starts at `_start` with the address of its IPC buffer in the first
argument register, which is where the kernel put it. That address is the
whole of what `Gate` holds, and the forty-two wrappers of the gate are the
system call table with names and types: each writes the call number and its
arguments into the buffer, executes `int 0x80`, and turns the status word
into a `Result`.

The address belongs to the thread, not to the process — the buffer of the
thread in slot `n` lies `n` pages below `IPC_BUFFER_TOP` — so a gate is a
value a thread carries and never a global.

`syscall` is one `int 0x80`. Everything a call says and answers stands in
the IPC buffer before and after it; no register carries anything.

This is an adapter crate on the allowlist of the safety policy.
