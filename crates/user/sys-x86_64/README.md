# user-sys-x86_64

What a user program cannot write in safe Rust: the entry point the kernel
jumps to, the instruction that makes a system call, the instruction that
stops a panicking thread, and the volatile access to a device window.

A thread starts at `_start` with the address of its IPC buffer in the first
argument register, which is where the kernel put it. That address is the
whole of what `Gate` holds, and the wrappers of the gate, one per entry of
`Syscall::ALL`, are the system call table with names and types: each
writes the call number and its arguments into the buffer, executes
`int 0x80`, and turns the status word into a `Result`.

The address belongs to the thread, not to the process — the buffer of the
thread in slot `n` lies `n` pages below `IPC_BUFFER_TOP` — so a gate is a
value a thread carries and never a global.

`syscall` is one `int 0x80`. Everything a call says and answers stands in
the IPC buffer before and after it; no register carries anything.

`stop` is one `ud2`: the panic handler of `program!` stops the thread with
an invalid-opcode fault, which the kernel reports to the fault handler of
the process (D-193).

`Mmio` makes one volatile read or write per access, at an offset
`user_rt::mmio::checked_offset` checks against the window and the width
(D-113).

This is an adapter crate on the allowlist of the safety policy.
