# AuDHSOS Planning Documents

AuDHSOS is a capability-based microkernel operating system written entirely
in Rust, without external code. This directory holds the planning documents.
They describe the target design, the rules the code must follow, and the
order in which the system is built.

Status: every entry of the decision register is decided; no open
decisions remain. Phases 0 to 8 of the roadmap are implemented and
released as 0.1.0; Phase 9 is next. The cryptography and TLS track of
document 11 is implemented through step T7 and R1 to R6; its integration
step T8 is not scheduled. Of the tracks of document 12, D1 to D9 of the
network stack, all of the shared foundations, the whole of track F, and
the tooling are implemented; what is left of document 12 is the two
integration steps, which are not scheduled.

## Reading order

| # | Document | Content |
|---|----------|---------|
| 1 | [Vision and scope](01-vision-and-scope.md) | Goals, non-goals, principles, glossary |
| 2 | [Architecture](02-architecture.md) | Kernel objects, memory, threads, IPC, system calls, boot sequence, userland |
| 3 | [Target platform](03-target-platform.md) | QEMU machine, devices, loader, boot image, boot information, test exit, HAL traits |
| 4 | [Safety policy](04-safety-policy.md) | Allowlist for `unsafe` and inline assembly, rules, inventory, enforcement |
| 5 | [Code organization](05-code-organization.md) | Workspace layout, crate catalog, layering, conventions, build automation |
| 6 | [Testing strategy](06-testing-strategy.md) | Test levels, harnesses, edge-case catalog, CI gates |
| 7 | [Toolchain and environment](07-toolchain-and-environment.md) | Toolchain pin, targets, QEMU on macOS, findings about the development machine |
| 8 | [Roadmap](08-roadmap.md) | Phases, deliverables, acceptance criteria, risks, resolved decisions |
| 9 | [Decision register](09-decisions.md) | Every binding decision as a statement |
| 10 | [Implementation plan](10-implementation-plan.md) | Exactly what to build in each remaining phase: crates, types, algorithms, tests, acceptance |
| 11 | [Cryptography and TLS](11-cryptography-and-tls.md) | The TLS 1.3 client track: primitives, certificates, protocol, tests, order of work |
| 12 | [Work parallel to the kernel phases](12-parallel-work.md) | The admission test for parallel work; the network stack, the shared foundations, the device logic, the tooling; what may be pulled forward |

Beside the documents lie the standards they cite, verbatim and with their
checksums, one directory per body that publishes them: [rfc/](rfc) for the
RFCs (D-59), [oasis/](oasis) for what OASIS publishes (D-100),
[w3c/](w3c), and [ecma/](ecma). Each directory has a `README.md` naming
what belongs there and how it is fetched. Nothing under them is compiled,
linked, or read at run time.

## Conventions for these documents

- All documents are written in English.
- A design document describes the target state. The roadmap describes the
  sequence in which that state is reached.
- The decision register lists what is decided. Documents state the target;
  they do not argue for it.
- Terms are defined once, in the glossary of the vision document, and used
  consistently everywhere else.
- When code and documents disagree, one of them is wrong. That is a bug and
  is fixed in the same change.
