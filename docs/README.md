# AuDHSOS Planning Documents

AuDHSOS is a capability-based microkernel operating system written entirely
in Rust, without external code. This directory holds the planning documents.
They describe the target design, the rules the code must follow, and the
order in which the system is built.

Status: every entry of the decision register is decided; no open
decisions remain. Phases 0 to 9 of the roadmap are implemented, Phases 0
to 8 released as 0.1.0, and Phase 10 is under way. Every side track of
documents 11 and 12 is finished: the TLS track of document 11 through
T7 and R1 to R6, and of document 12 the network stack D1 to D9, the shared
foundations, the whole of track F, and the tooling. What was left of both
documents is their two integration steps, and those are no longer
unscheduled: they are Phases 14 and 15, and document 13 specifies them
together with the two phases of kernel and bus work that has to come
first.

One track is begun and not finished: Secure Shell as a client, which
D-123 admits and [document 14](14-secure-shell-as-a-client.md)
specifies, as track S of the roadmap. Two steps of it exist, the wire
types and the binary packet of `audhsos-ssh` (S1) and the finite-field
arithmetic of `crypto-dh` (D-122), and one precondition is met: the
cipher's documents are in [openssh/](openssh) (D-134). Three questions
inside that track are open and each is a precondition of one of its steps
rather than of the track; section 14.13 names them.

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
| 13 | [The network on the machine](13-the-network-on-the-machine.md) | What has to exist before a network driver can be written: a clock and a deadline, entropy, MSI-X, PCI, DMA; then the driver, the server, and the socket protocol |
| 14 | [Secure Shell as a client](14-secure-shell-as-a-client.md) | The SSH-2 client: the algorithm set and what is refused, the crate, the three layers of the protocol, trusting a host key, testing against an implementation from outside |

Beside the documents lie the standards they cite, verbatim and with
their checksums, one directory per body that publishes them:
[rfc/](rfc) for the RFCs (D-59), [oasis/](oasis) for what OASIS
publishes (D-100), [w3c/](w3c), [ecma/](ecma), [itu/](itu) for the
JPEG Recommendations, [cipa/](cipa) for Exif, [ti/](ti) for the
16550 serial controller, which is a datasheet from a manufacturer rather
than a standard from a standards body, and [openssh/](openssh) for what
OpenSSH specified and no standards body did (D-134): the one cipher of
the Secure Shell client, kept as the OpenSSH document it came from and
the IETF draft that replaced it, and the private key format a client
reads a key of its own from. Each directory has a `README.md`
naming what belongs there and how it is fetched. Nothing under them is
compiled, linked, or read at run time.

D-124 states what decides whether a document is kept: whether it can be
obtained, not whether its licence permits the copy. `rfc/`, `oasis/`,
`w3c/`, `ecma/` and `openssh/` hold documents that may be redistributed.
`itu/`, `cipa/` and `ti/` hold documents that the ITU, CIPA and Texas
Instruments serve to anyone at no charge but do not licence for
redistribution; the copies are kept regardless, and each of those
READMEs quotes the restriction it stands against and states what follows
from it. `ti/` records one thing more, because the part the crate is
named after is the one document of all of these that its publisher no
longer serves: what is kept is the datasheet of a compatible part that
TI does serve, and the README says which routes to the original were
tried and what the substitution does and does not cover.
[pcisig/](pcisig) holds no document, because PCI-SIG releases the two
specifications the crate `pci` cites only to members or against payment;
it records instead which documents those are and what takes the place of
having them at hand.

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
