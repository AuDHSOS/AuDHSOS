# 8. Roadmap

The roadmap orders the work so that every phase ends with a system that
builds, passes its tests, and is documented. Sizes are relative (S, M, L,
XL) and describe effort, not calendar time.

## 8.1 Phase overview

| Phase | Name | Size | Ends with |
|-------|------|------|-----------|
| 0 | Project foundation | M | `cargo xtask check` passes on a configured workspace with the property-test engine and `kernel-types` |
| 1 | Memory management logic | L | memory map, frame allocator, page tables, mapper, address spaces, pools: complete and host-tested |
| 2 | Loader, boot, and test harness | XL | own UEFI loader boots test kernels in QEMU, which report over serial and exit with a status |
| 3 | Kernel memory bring-up | M | kernel reserve, pools, physical window, kernel stacks, address-space activation in QEMU |
| 4 | Interrupts and timer | M | APIC-driven timer ticks and line masking |
| 5 | Objects, threads, user mode, system calls | XL | user-mode threads issue system calls through the IPC buffer |
| 6 | IPC and interrupt forwarding | L | endpoints, notifications, fault handlers, device capabilities |
| 7 | Userland foundation | XL | root task, name server, console driver, memory server, hello application |
| 8 | Consolidation and release 0.1.0 | M | budget review, measurements, documentation refresh, tag |
| 9 | Framebuffer output | L | the userland display server draws through the framebuffer; the runner verifies pixels through QMP |
| 10 | PS/2 input | L | keyboard and pointer events reach a userland client; the runner injects them through QMP |
| 11 | Graphical demonstration | M | cursor, drawing, and typed text in `app-canvas`, verified end to end |

Every phase has the same definition of done: all catalog items for the
components in the phase have tests, `cargo xtask check` is green, the design
documents reflect the code, the changelog is updated. The
[implementation plan](10-implementation-plan.md) specifies the work of
each phase down to crates, types, algorithms, and tests.

Beside the phases run tracks that depend on none of them: the
cryptography and TLS crates of section 8.17, specified in
[document 11](11-cryptography-and-tls.md), and the tracks of sections
8.18 to 8.21, specified in [document 12](12-parallel-work.md). Section
8.22 states how many of them may be active at once and which phase work
may be pulled forward.

## 8.2 Phase 0: Project foundation

Status: implemented.

Deliverables:

- `LICENSE` with the AGPL-3.0 text verbatim; SPDX headers everywhere;
  `README.md`, `CONTRIBUTING.md`, `CHANGELOG.md`.
- `Cargo.toml` workspace with shared metadata and the lint set; the first
  crates: `audhsos-abi` (errors, rights, address constants), `kernel-types`
  (complete), `kernel-hal-api` (trait skeletons and doubles),
  `audhsos-sync`, `test-support` (property-test engine, model-test runner,
  first builders), `xtask`.
- `rust-toolchain.toml`, `.cargo/config.toml`, `rustfmt.toml`.
- xtask subcommands: `lint`, `check-layering`, `check-deps`,
  `unsafe-budget`, `test --host`, `coverage`, `miri`, `doc`, `check`; the
  policy tables; the toolchain verification at start.
- CI workflow with the host-only jobs.

Tests: catalog 6.6.1, 6.6.6 (rights items), 6.6.18, 6.6.19, and the xtask
items of 6.6.20 that exist at this point.

Acceptance: `cargo xtask check` passes locally and in CI; coverage of
`kernel-types` and `test-support` meets the thresholds; the first commit is
on `main`.

## 8.3 Phase 1: Memory management logic

Status: implemented.

Deliverables: memory map normalization, reserve selection, bitmap frame
allocator with contiguous allocation, `PageTableEntry`, `Mapper` over
`FrameAccess`, `TlbControl`, and `FrameSource`, address spaces with region
bookkeeping and quotas, generic `Pool<T>` with generation-checked ids and
reference counts, boot image header and boot information validation in
`audhsos-abi`.

Tests: catalog 6.6.2, 6.6.3, 6.6.4 including the model-based test, 6.6.5,
6.6.6 (pool items), 6.6.10.

Acceptance: every listed item has a test; coverage thresholds met; no QEMU
involved.

## 8.4 Phase 2: Loader, boot, and test harness

Status: implemented.

Deliverables: `audhsos-elf`; `audhsos-uefi`; `boot-uefi-x86_64` with file
loading, kernel placement, page-table construction through the mapper,
boot information, the naked entry function, diagnostics, and failure exit;
the disk image writer (GPT, FAT32, files) in the xtask; `kernel-hal-x86_64`
with the privileged instruction wrappers, GDT, TSS with double-fault stack,
IDT, exception handlers, `driver-uart16550` over direct port I/O, test
exit, boot information validation; `kernel-test-harness`;
`kernel-core::boot` generic over `Platform`, `Traps`, `DebugConsole`,
`TestExit`; xtask `image`, `run`, `qemu-runner`, `test --qemu`; CI QEMU
job.

Tests: catalog 6.6.13 (ELF items), 6.6.14, 6.6.15, 6.6.16 (descriptor
items), 6.6.17, 6.6.21 items boot, debug UART, exceptions (including the
`should_panic` double-fault kernel), and the loader failure images.

Acceptance: at least eight test kernels and three loader test images pass;
every `unsafe` block carries a `SAFETY:` comment; the policy table lists
the real counts.

## 8.5 Phase 3: Kernel memory bring-up

Status: implemented.

Deliverables: `PhysicalWindow` adapter with byte access, kernel reserve
from the boot information in the size the boot image header asks for,
adoption of the loader's page tables into the kernel region table,
removal of the identity mapping, kernel stack pool with guard pages,
`TlbControl` and `activate` adapters, and the object counts in
`kernel-core::config` that size the pools of Phase 5.

Tests: catalog 6.6.21 memory items.

## 8.6 Phase 4: Interrupts and timer

Deliverables: safe MADT parser; local APIC and I/O APIC register blocks and
adapters; legacy PIC masking; PIT-based timer calibration;
`InterruptController` and `Timer` traits with doubles; tick handling in
`kernel-core`.

Tests: catalog 6.6.11 with the fuzz target, 6.6.16 (APIC items), 6.6.21
interrupt items.

## 8.7 Phase 5: Objects, threads, user mode, system calls

Deliverables: handle tables, quotas, object types `Process`, `Thread`,
`MemoryObject`; scheduler; the context-switch naked function; user-mode
entry with synthesized frames; IPC buffer; system call vector `0x80`; the
`syscalls!` table in `audhsos-abi`; dispatcher and validation; system calls
for processes, threads, memory, and handles; `debug_log`; faults put
threads into `Faulted`. User-mode test programs are `user-sys-x86_64`
binaries embedded in test kernels as flat binaries.

Tests: catalog 6.6.6 (handle items), 6.6.7, 6.6.9, 6.6.21 thread, isolation,
and system call items.

## 8.8 Phase 6: IPC and interrupt forwarding

Deliverables: `Endpoint`, `Reply`, `Notification`, badges, handle transfer,
fault handler endpoints and fault messages, `Interrupt`, `IoPortRange`,
`SystemControl`, `Device` memory objects, `system_info`.

Tests: catalog 6.6.8, 6.6.21 IPC items.

## 8.9 Phase 7: Userland foundation

Deliverables: `user-sys-x86_64`, `user-rt` with the safe allocator,
`user-proto`, `user-loader`, `server-init`, `server-name`,
`server-console` over `driver-uart16550`, `server-memory`, `app-hello`;
boot image with tar archive; release build with `debug-uart` off.

Tests: catalog 6.6.12, 6.6.13 (tar items) with fuzz targets, 6.6.22, 6.6.23.

Acceptance: `cargo xtask run --release` prints the greeting through the
userland console driver; `cargo xtask test --e2e` passes.

## 8.10 Phase 8: Consolidation

Measure the system call round trip and the IPC round trip; decide the
`syscall` instruction path from the numbers. Review the `unsafe` budget.
Refresh every design document against the code. Write the 0.1.0 changelog
entry and tag.

## 8.11 Phase 9: Framebuffer output

Deliverables: `gfx` with pixel formats, fill, blit, clipping, damage
rectangles, double buffering, the project's bitmap font, and text
rendering; the display protocol in `user-proto` (info, surface creation
over shared memory objects, present with damage rectangles, cursor);
`server-display`; `server-init` creates the framebuffer `Device` memory
object from `system_info` and starts the display server; the xtask QMP
client with `screendump` and a PPM reader; `run --display`; the loader
test image without a VGA device.

Tests: catalog 6.6.24, 6.6.26, 6.6.27 (display items), 6.6.28, 6.6.29
(output items).

Acceptance: `cargo xtask test --e2e` verifies a filled rectangle and a
rendered string in a screendump; the test kernels boot with `-vga none`
and report an absent framebuffer.

## 8.12 Phase 10: PS/2 input

Deliverables: `driver-i8042` over the port access trait; the input
protocol in `user-proto` (key codes, key and pointer events, layout
tables `us` and `de`, subscription with a ring buffer and a
notification); `server-input` with the `IoPortRange` for ports `0x60`
to `0x64` and the `Interrupt` objects for lines 1 and 12; `server-init`
grants them; the xtask QMP client sends key and pointer events; fuzz
targets `scancode` and `mouse_packet`.

Tests: catalog 6.6.25, 6.6.27 (input items), 6.6.29 (input items).

Acceptance: a key sequence and a pointer path injected through QMP arrive
as events in a userland client and are echoed as `[input]` lines through
the console driver.

## 8.13 Phase 11: Graphical demonstration

Deliverables: `app-canvas` with a full-screen surface, a cursor that
follows the pointer, drawing while a button is held, and typed text
rendered with the bitmap font; the display server draws the cursor;
modifier handling and layout mapping in the input client library;
end-to-end tests that combine injected input with screendumps.

Tests: catalog 6.6.29 (combined items).

Acceptance: `cargo xtask run --display` shows the canvas; `cargo xtask
test --e2e` verifies cursor movement, a drawn stroke, and typed text in
screendumps.

## 8.14 Later work, not scheduled

virtio-blk driver and a file system server on top of the FAT32 logic of
8.20; virtio-net and a network server on top of the stack of 8.18, which
is what the TLS track of 8.17 is waiting for; RSA signature
verification with the bignum crate it needs, and certificate revocation
checking; virtio-gpu; virtio-input or `usb-tablet` for absolute pointer
coordinates; a compositor with several windows; the `aarch64` port under
HVF without a loader; SMP with per-CPU run queues; hardware port
permission bitmaps; kernel-object memory donation; an interface
definition language for protocols; recursive capability revocation; a
tickless timer; long file names in the disk image writer.

## 8.15 Risks

| Risk | Effect | Mitigation |
|------|--------|------------|
| A pinned nightly breaks a feature | build failures on update | update in an isolated commit; keep the previous pin until green; features limited to three |
| The loader's firmware interaction differs between QEMU's firmware builds | boot failures in CI but not locally, or the reverse | the loader uses seven boot services and five protocols only; the firmware version is recorded in the test log |
| The disk image writer and the property-test engine are tooling written before the kernel | Phase 0 and 2 grow | both are bounded by the catalog; no features beyond what the pipeline needs |
| `x86_64` details cost more than planned (APIC calibration, descriptor tables) | Phase 2 and 4 grow | scope is fixed to QEMU's default machine |
| Test time under TCG on Apple Silicon | slow feedback | many test cases per kernel, few kernels; host tests carry the bulk |
| `unsafe` creeps into logic crates | goal G4 fails | `forbid` at crate level plus the layering check; budget in CI |
| Pool sizing at boot is wrong for real workloads | spurious `PoolExhausted` | sizes are overridable in the boot image header; `system_info` exposes usage |
| Scope creep toward drivers and file systems before the userland foundation exists | first release slips | the roadmap order is binding; later work is listed, not scheduled |
| Full-screen copies under TCG are slow | a sluggish graphical demonstration | the display protocol requires damage rectangles; tests check correctness, not speed |
| The bitmap font is project-authored data | glyph errors, effort | 95 printable ASCII glyphs only; one checksum test per glyph |
| A relative PS/2 pointer needs a mouse grab in the QEMU window | awkward interactive use | absolute pointing through virtio-input or `usb-tablet` is listed as later work |
| The firmware's default mode and the framebuffer address vary between firmware builds | pixel tests fail on a different resolution | tests read the resolution from the boot information and never assume one; the loader reports the framebuffer as an `MmioReserved` region |
| Cryptography written from scratch has flaws that tests do not find | a connection that appears encrypted but is not | standards vectors, the RFC 8448 trace, negative tests for every rejection rule, fuzzing, a constant-time review section per crate, a verification-only asymmetric surface |
| The TLS track competes with the kernel phases for attention | phases slip | the track touches no kernel crate and has no phase dependency; it is worked on between phases, never instead of one |
| More than one side track is active at once | phases slip and no track finishes | at most one side track beside the cryptography track (D-45); document 12 fixes the order |
| The shared foundations of 8.19 arrive after their consumers | the same containers and time arithmetic are written twice | track E is scheduled before the tracks and phases that need it, and is small |

## 8.16 Resolved decisions

The two questions that were open before Phase 0 are decided in the
decision register: the project name and crate prefix (D-34) and the build
entry point on the development machine (D-35). No open decisions remain.

## 8.17 Track C: cryptography and TLS

Status: specified in [document 11](11-cryptography-and-tls.md); steps T1
and T2 are implemented, T3 is next.

The track prepares HTTPS for the day a network stack exists. Every crate
in it is pure logic without I/O or allocation, host-tested, and depends on
no kernel, loader, or userland crate. It therefore has no place in the
phase order and is built between phases.

| Step | Crates | Size | Ends with |
|------|--------|------|-----------|
| T1 | `crypto-ct`, `crypto-hash` | S | implemented: SHA-256, SHA-384/512, HMAC, HKDF against the standards vectors |
| T2 | `crypto-aead` | L | implemented: ChaCha20-Poly1305 and AES-GCM, both constant-time and table-free |
| T3 | `crypto-ec` | L | implemented: X25519, Ed25519 verification, P-256 ECDSA verification |
| T4 | `crypto-rng` | S | implemented: the ChaCha20 generator and the `Entropy` trait |
| T5 | `audhsos-der` | M | implemented: a strict DER reader; its fuzz target and its time conversion wait on document 12 (11.14) |
| T6 | `audhsos-x509` | L | certificate parsing, path validation, name matching, the test certificate builder |
| T7 | `audhsos-tls` | XL | the TLS 1.3 client reproduces the RFC 8448 trace byte for byte and completes a handshake against project-generated chains |
| T8 | integration | M | not scheduled: transport, the `random_bytes` system call, an HTTP client |

Definition of done per step, as for every phase: the catalog items of
6.6.30 to 6.6.38 that belong to the step have tests, `cargo xtask check`
is green, the documents reflect the code, the changelog is updated.

## 8.18 Track D: the network stack

Status: specified in [document 12](12-parallel-work.md), not started.

Sans-I/O logic crates that consume and produce frames, take time and
randomness as parameters, allocate nothing, and depend on no kernel,
loader, or userland crate. The driver and the server that will carry
their bytes are later work (8.14).

| Step | Crates | Size | Ends with |
|------|--------|------|-----------|
| D1 | `net-wire` | S | addresses, a bounds-checked cursor, and the internet checksum |
| D2 | `net-eth` | M | Ethernet II frames and an ARP cache with aging and retransmission |
| D3 | `net-ip` | M | IPv4 with reassembly, fragmentation, ICMP, and longest-prefix routing |
| D4 | `net-udp` | S | sockets, ephemeral ports, checksums |
| D5 | `net-tcp` | XL | the RFC 9293 state machine, RFC 6298 timers, and Reno congestion control, verified by two instances over a lossy network double |
| D6 | `net-dns`, `net-dhcp` | M | name resolution and address configuration as state machines |
| D7 | `net-http` | S | an HTTP/1.1 client that rejects the smuggling forms |
| D8 | `net-stack` | M | one interface, one `poll`, one `poll_at` |
| D9 | integration | - | not scheduled: virtio-net driver, network server, socket protocol, entropy system call, TLS transport (jointly with T8 of 8.17) |

Tests: catalog 6.6.42 to 6.6.50. Fuzz targets `ipv4`, `tcp_segment`,
`dns_message`, `http_response`.

## 8.19 Track E: shared foundations

Status: specified in [document 12](12-parallel-work.md), not started.

| Step | Crate | Size | Ends with |
|------|-------|------|-----------|
| E1 | `audhsos-time` | S | `UnixTime`, `CivilTime`, `Instant`, `Duration`, integer calendar arithmetic |
| E2 | `audhsos-encoding` | S | strict Base64, hex, and PEM without allocation |
| E3 | `audhsos-collections` | M | `ArrayVec`, `RingBuffer`, `BitSet`, `IndexList`, `IndexMap`, each model-tested |

Track E is scheduled first among the side tracks: step T5 of 8.17 needs
`UnixTime`, T6 needs PEM, track D needs all three, and phases 5 and 6
need `IndexList`.

Tests: catalog 6.6.39 to 6.6.41. Fuzz target `pem`.

## 8.20 Track F: device logic without devices

Status: specified in [document 12](12-parallel-work.md), not started.

| Step | Crate | Size | Ends with |
|------|-------|------|-----------|
| F1 | `virtio-queue` | M | split virtqueue and initialization state machine over a memory access trait |
| F2 | `fs-fat` | M | FAT32 read and write over a block device trait; the xtask image writer uses it |

Tests: catalog 6.6.51 and 6.6.52.

## 8.21 Track G: tooling

Status: specified in [document 12](12-parallel-work.md); G1 is
implemented, G2 is next.

| Step | Crate | Size | Ends with |
|------|-------|------|-----------|
| G1 | `fuzz-support` | S | implemented: the entry glue, the `fuzz_target!` macro, the corpus replay, the `elf`, `boot_image_header`, and `boot_info` targets, and `fuzz --regression` as a step of `check` |
| G2 | `audhsos-symbols` | M | the xtask resolves a panic address to function, file, and line |

G2 is worth having before Phase 3, because that is where kernel panics
begin to cost time. It was not, and Phase 3 is done; it is the next step
of this track.

Tests: catalog 6.6.53.

## 8.22 Capacity for parallel work

- At most one side track besides the cryptography track is active at a
  time (D-45).
- A side track is worked on between phases, never instead of one.
- Order: track E, then the cryptography track to T7, then track D, with
  step D5 not started beside an XL phase; track F when a driver becomes
  foreseeable; G2 before Phase 3.
- Phase work whose logic passes the admission test may be pulled
  forward without changing its phase, its catalog items, or its
  acceptance criteria: `gfx` (Phase 9), `driver-i8042` (Phase 10), the
  QMP client and PPM reader (Phase 9), the allocator logic of `user-rt`
  and the encodings of `user-proto` (Phase 7). Section 12.9 lists them.
