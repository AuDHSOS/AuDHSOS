# user-sys-x86_64 and user-rt audit findings

Repository: AuDHSOS/AuDHSOS. Audit of user-sys-x86_64, user-rt at main bc47df5.
Each `## ` section is one issue, filed 2026-09-19 with the number in its heading. `Title:` is the issue title, `Labels:` the labels, everything after `Body:` is the issue body verbatim.

---

## F01 — issue #99
Title: user-sys-x86_64: `Mmio::of` and `Mmio::window` let safe code make a misaligned volatile access
Labels: bug, part::userland
Body:
`Mmio::checked` at `crates/user/sys-x86_64/src/mmio.rs:187-197` tests the offset against the width of `T` and never the address of `base`. `Mmio::of` at `crates/user/sys-x86_64/src/mmio.rs:53-58` is a safe function over any `&mut [u8]`, and `Mmio::window` at `crates/user/sys-x86_64/src/mmio.rs:160-170` builds a window whose `base` is `self.base + offset` for any `offset` inside the window. The SAFETY comments of the accessors at `crates/user/sys-x86_64/src/mmio.rs:84-88` state that the offset being a multiple of the width makes the pointer aligned, which holds only for an aligned `base`. The precondition of `Mmio::new` at `crates/user/sys-x86_64/src/mmio.rs:36-40` names no alignment of `base`. `core::ptr::read_volatile` and `write_volatile` require an aligned pointer (`library/core/src/ptr/mod.rs:2124` of the toolchain `nightly-2026-08-25`).

Safe code produces undefined behavior: `Mmio::of(&mut page[1..]).read_u64(0)` reads a `u64` at an address congruent to 1 modulo 8, and `mmio.window(4, 8)?.read_u64(0)` on a page-aligned window reads a `u64` at an address congruent to 4 modulo 8. The two callers in the tree, `crates/user/programs/src/registers.rs:47` and `crates/user/net-programs/src/net_registers.rs:44`, pass page-aligned mappings, so no program of the tree triggers it today.

Fix: check `self.base.wrapping_add(offset).addr() % width == 0` in `checked`, so an accessor answers `None` for a misaligned address as it does for one outside the window; documenting an alignment precondition on `of` is not possible, because a safe function carries none.

---

## F02 — issue #101
Title: user-sys-x86_64: the pointers to the IPC buffer are built with `without_provenance`, which the core documentation defines as undefined for a memory access
Labels: bug, part::userland
Body:
`buffer` at `crates/user/sys-x86_64/src/lib.rs:51-56`, `Gate::reader` at `crates/user/sys-x86_64/src/gate.rs:229-236`, and `Gate::writer` at `crates/user/sys-x86_64/src/gate.rs:240-245` build the pointer to the IPC buffer with `core::ptr::without_provenance` or `without_provenance_mut` and dereference it. The documentation of `without_provenance_mut` states that a non-zero-sized memory access through a no-provenance pointer is undefined behavior (`library/core/src/ptr/mod.rs:949-951` of the toolchain `nightly-2026-08-25`). Memory outside the abstract machine, such as the page the kernel mapped, is reached through `with_exposed_provenance` (`library/core/src/ptr/mod.rs:383-385`).

Every system call of every user program runs through these three functions, so every build of the userland carries the undefined behavior; the SAFETY comments at `crates/user/sys-x86_64/src/lib.rs:53-55` and `crates/user/sys-x86_64/src/gate.rs:231-235` state the mapping and the exclusivity and say nothing of the provenance. Miri flags the access; LLVM compiles it as intended today.

Fix: replace `without_provenance` with `with_exposed_provenance` and `without_provenance_mut` with `with_exposed_provenance_mut` at the three sites, and name the provenance in the SAFETY comments; the same pattern stands in `kernel-hal-x86_64`, `boot-uefi-x86_64`, `user-programs`, and `user-test-programs`, which are outside this audit.

---

## F03 — issue #102
Title: user-sys-x86_64: `log_to_kernel` takes the count of the last word from the length before the cut
Labels: bug, part::userland
Body:
`log_to_kernel` at `crates/user/sys-x86_64/src/log.rs:63` cuts the line to `MAX_MESSAGE_WORDS` words, and `crates/user/sys-x86_64/src/log.rs:64-67` computes `trailing` from `line.len()` before the cut. The kernel prints `trailing` bytes of the last word (`crates/kernel/syscall/src/calls/debug.rs:40-45`, `crates/kernel/syscall/src/calls/debug.rs:57-62`).

`write_line(gate, None, &[b'x'; 3841])` writes 3840 bytes into the message area, sets the label to 1, and the kernel prints 3833 bytes: the seven bytes 3833 to 3839 stand in the buffer and reach the console never. The endpoint path of the same function refuses a line above `MAX_BYTES` with `CodecError::Full` (`crates/user/rt/src/message.rs:156-162`), so the two channels of `write_line` at `crates/user/sys-x86_64/src/log.rs:39-56` cut a long line at different lengths and one of them silently.

Fix: refuse a line longer than `MAX_MESSAGE_BYTES` with `Error::BufferTooSmall` before writing, as the endpoint path does; computing `trailing` from the cut length keeps the silent cut and is not taken.

---

## F04 — issue #104
Title: user-sys-x86_64: the allowlist row of the safety policy omits the MMIO accessor
Labels: bug, part::userland
Body:
The row for `user-sys-x86_64` in `docs/04-safety-policy.md:38` lists three contents that need `unsafe`: the trap instruction, `_start`, and the IPC buffer as a reference. `crates/user/sys-x86_64/src/mmio.rs:53-170` holds ten `unsafe` blocks for volatile reads and writes of a device window and for two window constructors, decided in D-113 (`docs/09-decisions.md:123`) and budgeted in `crates/tools/xtask/src/policy.rs:855-860`.

A reviewer who applies the checklist of section 4.9 against the row of 4.3 finds ten `unsafe` blocks the policy names no purpose for.

Fix: add "the volatile reads and writes of a mapped device window and the two constructors of that window (D-113)" to the row at `docs/04-safety-policy.md:38`.

---

## F05 — issue #106
Title: user-sys-x86_64: the README counts forty-two wrappers, the gate has fifty-one
Labels: bug, part::userland
Body:
`crates/user/sys-x86_64/README.md:8` states "the forty-two wrappers of the gate are the system call table". `COVERED` at `crates/user/sys-x86_64/src/gate.rs:139-191` holds fifty-one entries, the constant assertion at `crates/user/sys-x86_64/src/gate.rs:218-221` holds it to `Syscall::ALL`, and `impl Gate` carries fifty-one `pub fn` wrappers. `docs/10-implementation-plan.md:2234` carries the same count of forty-two.

A reader who checks the wrappers against the README's count finds nine methods the README does not account for.

Fix: replace the number in `crates/user/sys-x86_64/README.md:8` and `docs/10-implementation-plan.md:2234` with a reference to `Syscall::ALL`, so the sentence stays true when the table grows; the decision register entries D-92 and D-98 record the count of their day and stay.

---

## F06 — issue #107
Title: user-sys-x86_64: the module documentation of `mmio` claims a host test of the bound that does not exist
Labels: bug, part::userland
Body:
`crates/user/sys-x86_64/src/mmio.rs:15-16` states "the bound is tested on the host (D-113)". The crate holds no test module (`crates/user/sys-x86_64/src/` carries `gate.rs`, `lib.rs`, `log.rs`, `mmio.rs`), its target is `x86_64-unknown-none` alone, and its host tests run "through the programs of `user-test-programs` in QEMU" (`docs/05-code-organization.md:184`). No test in the workspace names `Mmio`.

A reviewer who relies on the sentence to skip the bound check of `checked` at `crates/user/sys-x86_64/src/mmio.rs:187-197` reviews a function no test covers, which is how F01 stands unnoticed.

Fix: move the bound arithmetic of `checked` into a pure function of `user-rt` that answers the offset or `None`, test it there with the cases of F01, and have `mmio.rs` call it; deleting the sentence is the option not taken, because the bound is the whole precondition of ten `unsafe` blocks.

---

## F07 — issue #109
Title: user-sys-x86_64: `_start` is entered with a stack pointer the `sysv64` convention does not allow
Labels: enhancement, part::userland
Body:
`program!` and `entry!` declare `_start` as `extern "sysv64"` (`crates/user/sys-x86_64/src/lib.rs:152`, `crates/user/sys-x86_64/src/lib.rs:193`). The System V AMD64 psABI, section 3.2.2, requires `%rsp + 8` to be a multiple of 16 at the entry of a function; no copy of the psABI stands under `docs/`. The kernel loads the stack pointer of a new thread with the value it was given (`crates/kernel/x86-tables/src/context.rs:98`), which for the root task is the page-aligned `STACK_TOP` (`crates/kernel/core/src/root.rs:60`, `crates/kernel/core/src/root.rs:157`), so `%rsp` is a multiple of 16 at the entry of `_start` and the compiler's assumption is off by eight bytes.

The target `x86_64-unknown-none` builds with `-sse,-sse2,...,+soft-float`, so the generated code holds no instruction that faults on a stack slot misaligned by eight, and no program of the tree faults today. A build with SSE enabled, or a compiler that places a 16-byte-aligned local relative to the entry `%rsp`, faults at the first such access in `_start` or in what it calls.

Fix: write `user_stack - 8` into the frame at `crates/kernel/x86-tables/src/context.rs:98`, so a thread starts as if a `call` had pushed a return address; making `_start` a naked function that aligns `%rsp` itself costs a second `asm!` site of `user-sys-x86_64` and is not taken.

---

## F08 — issue #111
Title: user-sys-x86_64: `Mmio::window` has no caller
Labels: enhancement, part::userland
Body:
`Mmio::window` at `crates/user/sys-x86_64/src/mmio.rs:160-170` holds one `unsafe` block and is called from no crate of the workspace; `crates/user/programs/src/registers.rs:64-73` and `crates/user/net-programs/src/net_registers.rs` reach a structure by adding its offset to the access offset instead.

The block counts against the budget of 32 at `crates/tools/xtask/src/policy.rs:859`, whose comment at `crates/tools/xtask/src/policy.rs:855-857` names the sub-window as a reason for the raise, and it is the second path of F01.

Fix: remove `Mmio::window` and lower the budget by one; keeping it for a future driver leaves an untested `unsafe` block in the crate.

---

## F09 — issue #112
Title: user-sys-x86_64: the panic handler of `program!` spins instead of stopping the thread
Labels: enhancement, part::userland
Body:
The panic handler written by `program!` at `crates/user/sys-x86_64/src/lib.rs:162-165` is `loop {}`. The macro documentation at `crates/user/sys-x86_64/src/lib.rs:128` states "It halts".

A thread that reaches the handler stays runnable and consumes every time slice the scheduler gives it for the rest of the run; `thread_info` reports it with no fault, and the fault handler of its process receives no message, so no server of the system learns that the thread is gone. `copy_from_slice` and the formatting machinery of `core` panic on their own conditions, so the lint set named at `crates/user/sys-x86_64/src/lib.rs:129-130` does not make the handler unreachable.

Fix: execute `ud2` in the handler through a second `asm!` site, raised in the budget with a decision register entry, so the kernel reports an undefined-instruction fault to the fault handler of the process and `thread_info` shows the thread stopped; correcting "halts" to "spins" in the documentation alone is not taken.
