# user-sys-x86_64

The two things a user program cannot write in safe Rust: the entry point
the kernel jumps to, and the instruction that makes a system call.

A thread starts at `_start` with the address of its IPC buffer in the
first argument register, which is where the kernel put it. `_start` hands
that address to the program's `main` and ends the thread when `main`
returns.

`syscall` is one `int 0x80`. Everything a call says and answers stands in
the IPC buffer before and after it; no register carries anything.

This is an adapter crate on the allowlist of the safety policy.
