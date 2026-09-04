# AuDHSOS Planning Documents

AuDHSOS is a capability-based microkernel operating system written entirely
in Rust, without external code. This directory holds the planning documents.
They describe the target design, the rules the code must follow, and the
order in which the system is built.

Status: every entry of the decision register is decided. Phase 0 of the
roadmap is implemented; Phase 1 is next. The remaining items in
[08-roadmap.md](08-roadmap.md#813-open-decisions) use their defaults.

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
| 8 | [Roadmap](08-roadmap.md) | Phases, deliverables, acceptance criteria, risks, open decisions |
| 9 | [Decision register](09-decisions.md) | Every binding decision as a statement |
| 10 | [Implementation plan](10-implementation-plan.md) | Exactly what to build in each remaining phase: crates, types, algorithms, tests, acceptance |

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
