# 4. Safety Policy

This document defines where `unsafe` code and inline assembly may exist,
how everything else stays safe, and how the rules are enforced by tooling.

## 4.1 The limit

A kernel cannot be written with zero `unsafe`: loading a page-table root,
switching stacks, acknowledging an interrupt, calling firmware, and reading a
device register are operations the Rust language can only express as
`unsafe`. The requirement "fully without unsafe memory" is implemented as:

1. Zero `unsafe` in every crate that contains logic, data structures, or
   policy. This includes all of memory management, the ELF parser, the UART
   register logic, and the UEFI structure definitions.
2. `unsafe` only in adapter crates whose sole job is to turn typed,
   validated values into hardware or firmware operations.
3. Every `unsafe` block is documented, counted, budgeted, reviewed, and where
   the host can execute it, run under Miri.

## 4.2 Definitions

- **Unsafe code**: any `unsafe` block, `unsafe fn` body, `unsafe impl`, or
  `unsafe trait`.
- **Assembly**: `asm!`, `naked_asm!`, `global_asm!`, `.S` or `.s` files,
  and build scripts that invoke an assembler.
- **Adapter crate**: a crate on the allowlist in 4.3.
- **Logic crate**: every other crate in the workspace.

## 4.3 Allowlist

| Crate | Content that needs `unsafe` | Assembly |
|-------|-----------------------------|----------|
| `kernel-hal-x86_64` | privileged registers, descriptor table loading, page-table memory through the physical window, MMIO for the APICs, port I/O, boot information validation from a raw pointer, context switch, the entry point and the exception triggers of a kernel test image | privileged instruction wrappers, one naked function, the exceptions a test image raises |
| `boot-uefi-x86_64` | firmware calls through function pointers, memory map buffer from a raw pointer, page-table memory through the identity mapping, `CR3` write, kernel entry | `CR3` write, port write for the exit device, one naked function |
| `audhsos-sync` | `Global<T>`: a `Sync` cell with a runtime borrow flag for kernel and userland global state | none |
| `user-sys-x86_64` | the system call trap instruction, `_start`, the `GlobalAlloc` adapter | one `asm!` statement: `int 0x80` |
| `fuzz-support` (host only) | the `LLVMFuzzerTestOneInput` entry that turns the fuzzer's pointer and length into a byte slice | none |

No other crate may contain `unsafe`. Adding a crate to this list requires a
new entry in the decision register.

## 4.4 Rules

| Rule | Statement | Enforced by |
|------|-----------|-------------|
| R1 | Every logic crate has `#![forbid(unsafe_code)]` at its root. | `cargo xtask check-layering` reads every crate root; workspace lint `unsafe_code = "deny"` as a second net |
| R2 | Adapter crates use `#![deny(unsafe_op_in_unsafe_fn)]`; every `unsafe` block is preceded by a `// SAFETY:` comment; one unsafe operation per block. | `clippy::undocumented_unsafe_blocks`, `clippy::multiple_unsafe_ops_per_block` at `deny` |
| R3 | No assembly files, no `global_asm!`, no assembler invoked from build scripts. | `cargo xtask check-layering` scans the tree |
| R4 | The number of `unsafe` blocks and `asm!` sites per adapter crate has a budget in the xtask policy table. Raising a budget requires a decision register entry referenced in the commit message. | `cargo xtask unsafe-budget` |
| R5 | Memory management logic (`kernel-mm`) contains neither `unsafe` nor assembly. The only hardware-touching operations are behind `FrameAccess`, `TlbControl`, and `activate`. | R1 plus the layering rule that `kernel-mm` may not depend on any adapter crate |
| R6 | The kernel never dereferences a user-supplied address. User data is reached only through IPC buffer frames that the kernel owns a reference to. | design of the system call interface; reviewed per system call |
| R7 | No panics on user-controllable paths. `unwrap`, `expect`, `panic!`, `todo!`, `unimplemented!`, `unreachable!`, slice indexing, implicit arithmetic overflow, and `as` casts are denied by lint; each justified exception uses `#[expect(lint, reason = "...")]`. | `clippy::unwrap_used`, `clippy::expect_used`, `clippy::panic`, `clippy::todo`, `clippy::unimplemented`, `clippy::unreachable`, `clippy::indexing_slicing`, `clippy::arithmetic_side_effects`, `clippy::as_conversions` at `deny` |
| R8 | No external code. `Cargo.lock` lists only workspace members. No `[dependencies]`, `[dev-dependencies]`, or `[build-dependencies]` entry points outside the workspace. No external Cargo subcommand is used by the xtask or CI. | `cargo xtask check-deps` parses `Cargo.lock` and every manifest |
| R9 | Unsafe code that can run on the host (`audhsos-sync`, the `GlobalAlloc` adapter, `fuzz-support`) runs under Miri in CI. | `cargo miri test` job |
| R10 | A change touching an adapter crate needs a review with the checklist in 4.9. | pull request template |

## 4.5 Inline assembly inventory

Every site is a one-line `asm!` wrapper unless marked as a naked function.
The xtask policy table holds the machine-readable form.

| Crate | Site | Instructions |
|-------|------|--------------|
| `kernel-hal-x86_64` | interrupt disable, enable, halt | `cli`, `sti`, `hlt` |
| `kernel-hal-x86_64` | read and write `CR3`, read `CR2` | `mov` from and to control registers |
| `kernel-hal-x86_64` | flush one page | `invlpg` |
| `kernel-hal-x86_64` | descriptor table loading | `lgdt`, `lidt`, `ltr` |
| `kernel-hal-x86_64` | segment register reload after `lgdt` | `mov` to data segment registers, far return for `CS` |
| `kernel-hal-x86_64` | flags register | `pushfq`, `pop` |
| `kernel-hal-x86_64` | model-specific registers (Phase 4) | `rdmsr`, `wrmsr` |
| `kernel-hal-x86_64` | port I/O, byte and double word so far | `in`, `out` |
| `kernel-hal-x86_64` | context switch (naked function, Phase 5) | save callee-saved registers, swap stack pointer, restore, return |
| `kernel-hal-x86_64` | the exceptions a test image raises (`testing`, features `debug-uart` and `test-exit`) | `int3`, `ud2`, `div` by zero, `mov` of a selector beyond the table into a segment register |
| `boot-uefi-x86_64` | kernel entry (naked function) | disable interrupts, write `CR3`, load stack pointer, jump |
| `boot-uefi-x86_64` | exit device on loader failure | `out` |
| `user-sys-x86_64` | system call trap | `int 0x80` |

Interrupt and exception entry use the `x86-interrupt` ABI, which the
compiler implements. System call entry is an interrupt vector and uses the
same ABI.

## 4.6 How the hard parts stay safe

| Part | Technique |
|------|-----------|
| Page tables | The walker is generic over `FrameAccess`, which returns `&mut PageTable` for a `PhysFrame`. The kernel adapter builds that reference from the physical window; the loader adapter builds it from the identity mapping; the test double from a `HashMap`. The walker never sees a pointer. |
| Physical memory window | `PhysicalWindow::frame_bytes_mut(frame) -> &mut [u8; 4096]` is the single conversion from a physical frame to a byte slice. The safety argument: the window maps all RAM, the frame is inside RAM by construction of `PhysFrame`, the kernel is single-threaded and non-preemptible, and callers hold the slice only inside one system call. |
| Descriptor tables | GDT entries, IDT entries, and the TSS are `repr(C)` types built by safe bit packing that is unit-tested for layout on the host. Only the three load instructions are `unsafe`. |
| APIC register blocks | `repr(C)` register layouts in a logic module, unit-tested for offsets. The adapter obtains one `&mut` to the block through the physical window and uses volatile field access. |
| ACPI tables and UEFI structures | The adapters hand table bytes to safe parsers as `&[u8]`. `audhsos-uefi` defines structure layouts only; the loader performs the calls. |
| Kernel stacks and thread entry | The initial user context is a sequence of `u64` values written into a `&mut [u64]` slice of the thread's kernel stack. The adapter only sets the stack pointer. |
| Object storage | Typed pools with generation-checked ids replace reference-counted pointers. Intrusive queues use indices. |
| Allocators | The frame allocator and the userland heap allocator compute offsets. The userland `GlobalAlloc` adapter converts an offset to a pointer with `wrapping_add` on a base pointer obtained once at heap creation. |
| Global state | `audhsos-sync::Global<T>` holds the kernel state and the userland heap state. Access requires the interrupt guard in the kernel; a second concurrent borrow is detected by a flag and reported as a kernel bug. |
| User memory | Never dereferenced (R6). |
| Cryptographic secrets | The cryptography crates of document 11 are logic crates without `unsafe`. They branch and index on public values only, use no lookup tables in a primitive that sees a key, and hold key material in `Secret<N>`. The one thing safe Rust cannot promise is erasure: without `write_volatile` the `Drop` implementation overwrites and calls `black_box`, which is best effort. The limit is documented, not hidden. |

## 4.7 External code

There is none. The workspace depends on the Rust toolchain (`core`,
`alloc` for userland, `std` for host tools) and on QEMU with its bundled
UEFI firmware. Every parser, allocator, driver, lock, test generator, and
fuzz entry point is project code.

## 4.8 Enforcement summary

| Check | Command | Runs |
|-------|---------|------|
| Lints | `cargo xtask lint` (`rustfmt --check`, `clippy` with the workspace lint set, `deny(warnings)`, SPDX headers) | every push |
| Layering, forbids, assembly files | `cargo xtask check-layering` | every push |
| External code | `cargo xtask check-deps` | every push |
| Unsafe budget | `cargo xtask unsafe-budget` | every push |
| Miri | `cargo miri test` on the host-executable adapter crates | every push |
| Host tests, property tests, model-based tests | `cargo xtask test --host` | every push |
| Coverage | `cargo xtask coverage` using `-C instrument-coverage` and the `llvm-profdata` and `llvm-cov` binaries of the toolchain | every push |
| Loader, kernel, and end-to-end tests | `cargo xtask test --qemu`, `cargo xtask test --e2e` | every push |
| Fuzzing | `cargo xtask fuzz --all --time 60` using `-Zsanitizer=fuzzer` from the toolchain | nightly schedule; findings become regression tests |

## 4.9 Review checklist for adapter crates

1. Does the `SAFETY:` comment state every precondition and who guarantees it?
2. Could the operation be moved behind an existing trait instead of adding a
   new `unsafe` site?
3. Is the budget in the xtask policy table updated and the decision register
   entry referenced?
4. Is there a host test (with Miri where possible) or a QEMU test that
   exercises the new site, including its failure mode?
5. Does the change keep the trait crate architecture neutral?

## 4.10 Review checklist for cryptographic crates

The crates of document 11 contain no `unsafe`, so 4.9 does not apply to
them. They carry their own checklist, and each crate documents its answers
in the crate documentation.

1. Which functions receive a secret as an argument or hold one in their
   state?
2. For each of those: is every branch condition and every index a public
   value? A length, a protocol constant, and a certificate field are
   public; a key, a shared secret, a traffic secret, and plaintext are not.
3. Does any primitive that sees a key contain a lookup table? It must not.
4. Is every comparison of secret bytes `ct_eq` rather than `==`?
5. Does a failing authentication leave the output buffer without
   unauthenticated plaintext?
6. Is the new code covered by a vector test from the standard that defines
   it, and by a negative test for every rejection rule it adds?
