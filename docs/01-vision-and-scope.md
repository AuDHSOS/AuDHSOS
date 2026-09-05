# 1. Vision and Scope

## 1.1 What AuDHSOS is

AuDHSOS is a microkernel operating system.

- The kernel provides exactly five mechanisms: address spaces, threads with
  scheduling, inter-process communication (IPC), capabilities, and interrupt
  forwarding.
- Every policy and every driver lives in userland: device drivers, file
  systems, networking, memory allocation policy, program loading, naming, and
  system services.
- The system is written in Rust. There are no assembly files. Inline
  assembly and `unsafe` code exist only in small, audited adapter crates.
- The system contains no external code. Every line in the boot loader, the
  kernel, the userland, the build automation, and the test tooling belongs
  to this repository. The only external components are the Rust toolchain
  and QEMU.
- The first target is the QEMU `x86_64` virtual machine booted through UEFI.
  Further architectures follow behind a stable hardware abstraction
  boundary.
- The project is licensed under the GNU Affero General Public License,
  version 3 only (`AGPL-3.0-only`), with no exceptions of any kind.

## 1.2 Goals

| ID | Goal | Measured by |
|----|------|-------------|
| G1 | Minimal kernel | The kernel implements only the five mechanisms above. The split between kernel and userland is fixed in [Architecture 2.2](02-architecture.md#22-what-runs-where). |
| G2 | Maximal userland | All drivers, including the console driver, run as unprivileged userland processes once the root task runs. Kernel debug output is a build-time feature that is off in release builds. |
| G3 | Pure Rust | No `.S` files, no `global_asm!`. Memory management contains no assembly. Inline assembly is confined to adapter crates and inventoried in the [safety policy](04-safety-policy.md#45-inline-assembly-inventory). |
| G4 | Memory safety | Every crate carries `#![forbid(unsafe_code)]` except the crates on the adapter allowlist. Every `unsafe` block has a `SAFETY:` comment. The number of `unsafe` blocks is budgeted and checked in CI. |
| G5 | No external code | `Cargo.lock` lists only workspace members. No crates.io, git, path, or vendored third-party code in any dependency section of any crate, including build and dev dependencies. No external Cargo subcommands. |
| G6 | State-of-the-art organization | One Cargo workspace, enforced dependency layering, shared behavior expressed through traits and generics, no copy-pasted logic, complete API documentation. |
| G7 | Complete testing | Every safe logic crate is unit- and property-tested on the host. Every hardware-facing path is tested inside QEMU. The [edge-case catalog](06-testing-strategy.md#66-edge-case-catalog) is part of the definition of done for each component. |
| G8 | English only | Code, comments, documentation, commit messages, and test names are in English. |
| G9 | AGPL-3.0-only | Every source file has an SPDX header. |
| G10 | QEMU first | The system boots, runs its tests, and exits with a status code on an unmodified QEMU installation with its bundled UEFI firmware. |

## 1.3 Non-goals

These are out of scope until the roadmap says otherwise. The design must not
prevent them, but no effort is spent on them now.

- POSIX or Linux binary compatibility.
- Sound.
- Graphics output and input devices in the first release; they follow in
  Phases 9 to 11 of the roadmap.
- Booting on physical hardware.
- Legacy BIOS boot.
- Symmetric multiprocessing (SMP) in the first release.
- Networking and persistent storage in the first release.
- Performance tuning beyond what correctness requires.
- Formal verification or security certification.

## 1.4 Principles

When goals conflict, these principles decide, in this order.

1. **Mechanism in the kernel, policy in userland.** If a feature can be
   implemented in userland at acceptable cost, it is implemented in userland.
2. **Safety over convenience.** A design that needs less `unsafe` wins over a
   design that is shorter or faster.
3. **Own code over external code.** A component is written in this
   repository, tested here, and audited here.
4. **Logic is separated from hardware.** Every algorithm that does not need
   privileged instructions is written against a trait and is testable on the
   host. Hardware access is an adapter behind that trait.
5. **Invariants are types.** Alignment, address ranges, rights, and object
   types are encoded in the type system so that invalid states cannot be
   constructed.
6. **No untested behavior.** A change without tests for its edge cases is not
   done.
7. **No duplication.** Repeated logic is factored into a shared crate, a
   generic, or a declarative table. Copy-paste is a review blocker.
8. **Every significant decision is recorded.** The decision register holds
   the decision as a statement. Reversing a decision means a new entry that
   supersedes the old one.

## 1.5 Glossary

| Term | Meaning in this project |
|------|-------------------------|
| Loader | The UEFI application in this repository that loads the kernel and the boot image, builds the initial page tables, and enters the kernel. |
| Kernel | The privileged component running in ring 0. Provides mechanisms only. |
| HAL | Hardware abstraction layer. An architecture-neutral trait crate plus one adapter crate per architecture. |
| Adapter crate | A crate on the allowlist of the safety policy. The only place where `unsafe` and inline assembly are allowed. |
| Logic crate | Every crate that is not an adapter crate. Carries `#![forbid(unsafe_code)]`. |
| Userland | Everything that runs in ring 3: the root task, servers, drivers, applications. |
| Root task | The first userland process. It receives every capability the kernel created at boot and starts the rest of the system. |
| Server | A userland process that offers a service over IPC (name server, console server, memory server). |
| Driver | A server that owns a device through interrupt, I/O port, or device memory capabilities. |
| Kernel object | An entity managed by the kernel and referenced only through capabilities: process, thread, memory object, endpoint, reply, notification, interrupt, I/O port range, system control. |
| Capability | An unforgeable reference to a kernel object together with a set of rights and an optional badge. In userland a capability is addressed by a handle. |
| Handle | A per-process integer that names a capability in the process's handle table. |
| Rights | The set of operations a capability permits. Rights can only be reduced, never increased. |
| Badge | A 64-bit value attached to an endpoint capability. The receiver of a message sees the badge and uses it to identify the sender. |
| Endpoint | A kernel object for synchronous message passing. |
| Notification | A kernel object carrying a word of signal bits for asynchronous wake-ups. |
| Memory object | A kernel object representing a contiguous range of physical frames that can be mapped into address spaces. |
| Frame | A physical page of 4 KiB. |
| Page | A virtual page of 4 KiB. |
| IPC buffer | One page per thread, shared between the thread and the kernel. Carries system call arguments, results, and message payloads. |
| Fault handler | The endpoint that receives a message when a thread faults. |
| Kernel reserve | The physical memory the kernel keeps for itself at boot: its object pools, page tables, and kernel stacks. |
| Boot information | The structure the loader hands to the kernel: memory regions, physical window offset, kernel and boot image locations, ACPI root pointer. |
| Boot image | The file the loader places in memory next to the kernel. It contains a fixed header, the root task as a flat binary, and a tar archive with the remaining userland. |
| Disk image | The GPT-partitioned disk QEMU boots from, with one FAT32 EFI system partition. Contains the loader, the kernel, and the boot image. |
| Decision register | The list of binding decisions in [09-decisions.md](09-decisions.md). |
| xtask | The project's build automation, implemented as a Rust binary invoked with `cargo xtask`. |
