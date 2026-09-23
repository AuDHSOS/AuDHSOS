# Reference documents: the x86-64 psABI

The calling convention `extern "sysv64"` compiles to, kept verbatim so that
a stack or register rule can be checked against its source without a
network.

| File | Document | Retrieved | Bytes | SHA-256 |
|------|----------|-----------|-------|---------|
| `x86-64-psABI-1.0.pdf` | *System V Application Binary Interface, AMD64 Architecture Processor Supplement (With LP64 and ILP32 Programming Models)*, Version 1.0, H.J. Lu, M. Matz, M. Girkar, J. Hubička, A. Jaeger, M. Mitchell (eds.), 12 March 2025, 154 pages, built from commit `e1ce0983` | 2026-09-23 from `https://gitlab.com/x86-psABIs/x86-64-ABI/-/jobs/artifacts/master/raw/x86-64-ABI/abi.pdf?job=build` | 510003 | `19f346cc723dbeb6ad80494614291f4e85aba3caca48dcfd537d33f82af6d6ac` |

## Terms

The repository `x86-psABIs/x86-64-ABI` carries no licence. The maintainers
serve the PDF to anyone at no charge; D-124 keeps the copy regardless.

## Lookups

| Rule | Section | Page |
|------|---------|------|
| `(%rsp + 8)` is a multiple of 16 at a function entry | 3.2.2 The Stack Frame | 22–23 |

## Who cites it

`kernel-x86-tables`: `prepare_user` in `crates/kernel/x86-tables/src/context.rs`
starts a thread with the stack pointer 3.2.2 requires at the entry of `_start`
(D-193, issue #109).

Nothing here is compiled, linked, or read at run time.
