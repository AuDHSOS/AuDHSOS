# 8. Roadmap

The roadmap orders the work so that every phase ends with a system that
builds, passes its tests, and is documented. Sizes are relative (S, M, L,
XL) and describe effort, not calendar time.

## 8.1 Phase overview

| Phase | Name | Size | Ends with |
|-------|------|------|-----------|
| 0 | Project foundation | M | `sh tools/xtask-check.sh` passes on a configured workspace with the property-test engine and `kernel-types` |
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
components in the phase have tests, `sh tools/xtask-check.sh` is green, the
design documents reflect the code, the changelog is updated. The
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

Acceptance: `sh tools/xtask-check.sh` passes locally and `cargo xtask check`
in CI; coverage of `kernel-types` and `test-support` meets the thresholds;
the first commit is on `main`.

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
`kernel-core::boot` over `Platform`, `DebugConsole`, and `TestExit`; xtask `image`, `run`, `qemu-runner`, `test --qemu`; CI QEMU
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

Status: implemented.

Deliverables: safe MADT parser; local APIC and I/O APIC register blocks and
adapters; legacy PIC masking; PIT-based timer calibration;
`InterruptController` and `Timer` traits with doubles; tick handling in
`kernel-core`.

Tests: catalog 6.6.11 with the fuzz target, 6.6.16 (APIC items), 6.6.21
interrupt items.

## 8.7 Phase 5: Objects, threads, user mode, system calls

Status: implemented.

Deliverables: handle tables, quotas, object types `Process`, `Thread`,
`MemoryObject`; scheduler; the context-switch naked function; user-mode
entry with synthesized frames; IPC buffer; system call vector `0x80`; the
`syscalls!` table in `audhsos-abi`; dispatcher and validation; twenty of the
forty-one system calls the table held then, those for processes except the fault handler, for
threads, for memory, and for handles, plus `debug_log`; every other call
returns `Unsupported`; faults put threads into `Faulted`. User-mode test
programs are `user-sys-x86_64` binaries embedded in test kernels as flat
binaries.

Tests: catalog 6.6.6 (handle and pool items), 6.6.7, 6.6.9 for the calls
this phase implements, 6.6.21 address space, thread, and system call items
and the isolation item that names no handler.

## 8.8 Phase 6: IPC and interrupt forwarding

Status: implemented.

Deliverables: `Endpoint`, `Reply`, `Notification`, badges, handle transfer,
fault handler endpoints and fault messages including
`process_set_fault_handler`, `Interrupt`, `IoPortRange`, `SystemControl`,
`Device` memory objects, `system_info`; the twenty-one calls Phase 5 left
returning `Unsupported`.

Tests: catalog 6.6.8, 6.6.21 IPC items and the isolation item marked from
Phase 6.

## 8.9 Phase 7: Userland foundation

Status: implemented.

Deliverables: `user-sys-x86_64`, `user-rt` with the safe allocator,
`user-proto`, `user-loader`, `server-init`, `server-name`,
`server-console` over `driver-uart16550`, `server-memory`, `app-hello`,
`app-checks`, `app-faulter`; the kernel starts the root task from an ELF
of the boot image (D-92); boot image with tar archive; the kernel gives
COM1 up when the userland takes it, so the handover and not a feature
flag decides who writes.

Tests: catalog 6.6.12, 6.6.13 (tar items) with fuzz targets, 6.6.22,
6.6.23, 6.6.56, 6.6.57.

Acceptance: `sh tools/xtask.sh run --release` prints the greeting through
the userland console driver; `sh tools/xtask.sh test --e2e` passes and the
root task ends the machine itself (D-94); `check` runs it.

## 8.10 Phase 8: Consolidation

Status: implemented.

Measure the system call round trip and the IPC round trip; decide the
`syscall` instruction path from the numbers. Review the `unsafe` budget.
Refresh every design document against the code. Write the 0.1.0 changelog
entry and tag.

### 8.10.1 The measurements

The `bench` test kernel measures both round trips. It hangs a hook on the
entry of the system call gate, which a round trip passes through exactly
once, and takes the difference between two entries of the same call made
by a thread that does nothing in between. The shorter round trip is a
`thread_yield` with one runnable thread: the trap, the dispatch of the
shortest call of the table, and the return to ring three. The longer one
is a call and its answer between two user threads over an endpoint, with
an empty message: the rendezvous and the two switches it takes.

The figures below are ticks of the time-stamp counter, from the reference
machine of 3.1.1 — QEMU without hardware virtualization, one processor,
`-cpu qemu64` — on the development machine of 7.5. The `[bench]` line
carries the median; the mean is the whole span of the run divided by the
round trips in it.

| Round trip | Profile | Median | Mean | Shortest | Longest |
|------------|---------|--------|------|----------|---------|
| system call (`thread_yield`) | release | 8 000 | 7 829 | 7 000 | 943 000 |
| call and reply (`ipc_call`) | release | 65 000 | 65 708 | 62 000 | 2 056 000 |
| system call (`thread_yield`) | debug | 72 000 | 74 468 | 70 000 | 4 219 000 |
| call and reply (`ipc_call`) | debug | 524 000 | 534 071 | 488 000 | 5 731 000 |

Three things these numbers are not. They are not a figure of hardware:
every instruction of the measured path is translated by the emulator, and
the ratio between two instructions there is not the ratio between them on
a processor. They are not repeatable to the digit: successive runs differ
by a few percent with the load of the host. And a single difference is
quantized: under this emulator the counter advances in steps of a thousand
ticks, which is an eighth of the shorter round trip in the release
profile — which is why the mean over the whole run is reported beside the
median, its error being the step divided by ten thousand.

What the numbers are good for is the comparison of the two with each
other, and of either with itself after a change of this kernel. The call
and reply costs about eight times a bare entry and return; whatever the
entry instruction is, it is a small part of the work of an IPC. [D-102](09-decisions.md)
decides the `syscall` instruction path from that.

`sh tools/xtask.sh test --qemu` runs the image in the debug profile and
prints both lines. The release figures were taken by building the same
image with `--release`.

## 8.11 Phase 9: Framebuffer output

Status: implemented.

Deliverables: `gfx` with pixel formats, fill, blit, clipping, damage
rectangles, double buffering, the project's bitmap font, and text
rendering; the display protocol in `user-proto` (info, surface creation
over shared memory objects, present with damage rectangles, cursor);
`server-display` and `app-paint`; `server-init` creates the framebuffer
`Device` memory object from `system_info` and starts the display server;
the xtask QMP client with `screendump`, a JSON subset, and a PPM reader;
`run --display`; the run of the whole system without a graphics adapter.

Tests: catalog 6.6.24, 6.6.26, 6.6.27 (display items), 6.6.28, 6.6.29
(output items).

Acceptance: `sh tools/xtask.sh test --e2e` verifies a filled rectangle and
a rendered string in a screendump of the running machine, and then runs the
same image with `-vga none`, where the kernel reports an absent
framebuffer, the display server reports no screen, and the run ends by
itself.

Done: the mode the firmware set reaches the display server as the value
roles of the startup message (D-103); a mapping of a full screen stays one
region (D-104); the reference machine draws on 1280 by 800 pixels in
`bgrx8888`.

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

Acceptance: `sh tools/xtask.sh run --display` shows the canvas;
`sh tools/xtask.sh test --e2e` verifies cursor movement, a drawn stroke,
and typed text in screendumps.

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
entry point on the development machine (D-35, amended by D-64, which puts
the wrapper scripts of `tools/` in front of the proxy). No open decisions
remain.

## 8.17 Track C: cryptography and TLS

Status: specified in [document 11](11-cryptography-and-tls.md); steps T1
to T7 and R1 to R6 are implemented and reviewed as a whole. T8 is the
integration and is not scheduled: it needs a transport from track D, the
`random_bytes` system call, and the driver and server that carry the
bytes. What the track is still waiting on, and who owns each piece, is
section 11.14.

R1 to R6 are RSA verification, specified in section 11.15 and
implemented. They are the one thing on this track that changed what the
system can reach rather than how well it is checked: without them a chain
that is RSA to the root cannot be walked, which is most of the public web.
They needed nothing from another track and were built between phases as T1
to T7 were.

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
| T5 | `audhsos-der` | M | implemented: a strict DER reader with its fuzz target; its time conversion waits on document 12 (11.14) |
| T6 | `audhsos-x509` | L | implemented: certificate parsing, path validation, name matching, the test certificate builder |
| T7 | `audhsos-tls` | XL | implemented: the client reproduces the RFC 8448 trace and completes a handshake against project-generated chains |
| T8 | integration | M | not scheduled: transport, the `random_bytes` system call, an HTTP client |
| R1 | `crypto-bignum` | M-L | implemented: the limb arithmetic moved out of `crypto-ec`, with a modulus known at run time and Montgomery exponentiation in a narrow and a wide form |
| R2 | `crypto-rsa` | M | implemented: the key with its bounds, and PKCS #1 v1.5 verified by construction (D-80) |
| R3 | `crypto-rsa` | M | implemented: MGF1 and EMSA-PSS-VERIFY |
| R4 | `audhsos-x509` | L | implemented: the RSA identifiers with the NULL parameter rule of RFC 4055, the key, and the test certificates |
| R5 | `audhsos-tls` | M | implemented: the six code points under the `CertificateVerify` rule of D-82, and the RFC 8448 signature verified |
| R6 | fuzzing and the probe | S-M | implemented: the `rsa` fuzz target, and three RSA-rooted hosts reached by `tools/tls-probe` |

Definition of done per step, as for every phase: the catalog items of
6.6.30 to 6.6.38 and 6.6.55 that belong to the step have tests,
`sh tools/xtask-check.sh` is green, the documents reflect the code, the
changelog is updated.

## 8.18 Track D: the network stack

Status: D1 to D9 implemented, which is every step but the integration.
D10 is specified in [document 12](12-parallel-work.md) and is not
scheduled.

Sans-I/O logic crates that consume and produce frames, take time and
randomness as parameters, allocate nothing, and depend on no kernel,
loader, or userland crate. The driver and the server that will carry
their bytes are later work (8.14).

| Step | Crates | Size | Ends with |
|------|--------|------|-----------|
| D1 | `net-wire` | S | implemented: the addresses of both families with one canonical text each, `IpAddr` and `IpCidr`, the `EtherType` and `Protocol` tables, a `Reader` and `Writer` that never leave their buffer, and the internet checksum of RFC 1071 with both pseudo-header forms |
| D2 | `net-eth` | M | implemented: Ethernet II frames with a receive filter that drops rather than reports, ARP over RFC 826, and one neighbor cache for both families with the five states and the schedule of RFC 4861 |
| D3 | `net-ip` | M | implemented: IPv4 with reassembly, fragmentation, `ICMPv4` under the restrictions of RFC 1122, a longest-prefix routing table over both families, and the send path that joins them to the neighbor cache |
| D4 | `net-ipv6` | L | implemented: the header and its extension chain bounded in headers and in bytes, `ICMPv6` summed over the pseudo-header, Neighbor Discovery into the cache of `net-eth`, router advertisements with SLAAC and the DNS servers of RFC 8106, duplicate address detection, and path MTU discovery in the send path |
| D5 | `net-udp` | S | implemented: datagrams under the checksum rule of each family, a fixed socket table with wildcard and address-specific bindings, ephemeral ports drawn as RFC 6056 asks, and a receive ring of self-describing records in the caller's memory |
| D6 | `net-tcp` | XL | implemented: the eleven states of RFC 9293 with active and passive open, the sequence arithmetic they are decided by, send and receive windows over caller-supplied rings with reassembly in place, the RFC 6298 timer with Karn's rule, Reno congestion control, delayed acknowledgments, a persist timer, the reset checks of RFC 5961, and a connection table that answers a segment to a closed port; two instances verified back to back over a network double that delays, duplicates, reorders, and drops |
| D7 | `net-dns`, `net-dhcp` | M | implemented: the RFC 1035 message format with name compression bounded three ways, a stub resolver that asks `A` and `AAAA` at once over `net-udp` with retry, server rotation and a deadline, alias chains followed across messages under one budget of eight; and the RFC 2131 client with the four-message exchange, the strict option walk of RFC 2132, and the lease timers with T1 renewal, T2 rebinding and expiry |
| D8 | `net-http` | S | implemented: the request writer with every field checked before a byte of it goes down, and an incremental response decoder that takes one line of the head per call, decides its framing once under RFC 9112 section 6.3, and refuses every message that two parsers could read differently |
| D9 | `net-stack` | L | implemented: one interface, one `poll`, one `poll_at`, generation-checked handles, an outgoing frame queue in the caller's memory, the demultiplexer down both families, DHCP and router advertisements wired to the address table and the routes, duplicate address detection, the resolver, and the address selection of RFC 6724 |
| D10 | integration | - | not scheduled: virtio-net driver, network server, socket protocol, entropy system call, TLS transport (jointly with T8 of 8.17) |

The stack carries IPv4 and IPv6 together (D-69), which supersedes the
first clause of D-50. An address is an `IpAddr` above `net-wire`, so the
transports, the resolver, and the facade are written once; what the second
family costs is the header format of D4 and its own address
configuration, not a second copy of everything above it.

Tests: catalog 6.6.42 to 6.6.50 and 6.6.54. Fuzz targets `ipv4`, `ipv6`,
`tcp_segment`, `dns_message`, `http_response`.

## 8.19 Track E: shared foundations

Status: implemented.

| Step | Crate | Size | Ends with |
|------|-------|------|-----------|
| E1 | `audhsos-time` | S | implemented: `UnixTime`, `CivilTime`, `Instant`, `Duration`, and the integer calendar of the proleptic Gregorian rule over the years 0 to 9999 |
| E2 | `audhsos-encoding` | S | implemented: strict Base64, hex, and PEM without allocation, with the fuzz target `pem` |
| E3 | `audhsos-collections` | M | implemented: `ArrayVec`, `RingBuffer`, `BitSet`, `IndexList` with `Link`, and `IndexMap`, each against a reference model |

Track E was scheduled first among the side tracks: step T5 of 8.17 needed
`UnixTime`, T6 needed PEM, track D needs all three, and phases 5 and 6
need `IndexList`. Track D is therefore unblocked from D1.

Tests: catalog 6.6.39 to 6.6.41. Fuzz target `pem`.

## 8.20 Track F: device logic without devices

Status: implemented.

| Step | Crate | Size | Ends with |
|------|-------|------|-----------|
| F1 | `virtio-queue` | M | implemented: the descriptor table, the two rings and the chain arithmetic over a memory access trait, with the free set in the queue's own memory rather than in the table the device can see; the initialization state machine with its two failure paths; no packed ring, no indirect descriptor and no `EVENT_IDX`, each refused by name at negotiation (D-52, D-99) |
| F2 | `fs-fat` | M | implemented: the boot parameter block against FAT12 and FAT16, cluster chains with every walk bounded by the cluster count, allocation and release with the free count counted at mount, directories in 8.3 form, and file read and write over a cursor; the xtask image writer is a user of it and keeps no FAT32 structure of its own (D-53, D-107) |

Tests: catalog 6.6.51 and 6.6.52.

## 8.21 Track G: tooling

Status: implemented.

| Step | Crate | Size | Ends with |
|------|-------|------|-----------|
| G1 | `fuzz-support` | S | implemented: the entry glue, the `fuzz_target!` macro, the corpus replay, the `elf`, `boot_image_header`, and `boot_info` targets, and `fuzz --regression` as a step of `check`. Track C added `der`, `x509`, `tls_record`, `tls_handshake`, and — with the RSA steps R1 to R6 — `rsa` on top of it |
| G2 | `audhsos-symbols` | M | implemented: the symbol table, the DWARF 4 and 5 line programs, `xtask symbolize`, and the automatic report on a failing QEMU run |
| G3 | `doc-markdown`, `doc-html`, `doc-svg`, `doc-pdf`, `docpdf`, `audhsos-deflate` | L | implemented: every document of `docs/` and every standard beside them as a PDF under `xtask pdf`, with a Markdown and an HTML parser, an SVG reader, a PDF 1.7 writer, and the DEFLATE compressor of RFC 1951 in the zlib wrapper of RFC 1950 that its streams are written through |

G2 was worth having before Phase 3, because that is where kernel panics
begin to cost time. It came after it instead, and is in place for
Phase 4. G3 came after Phase 7 and has no phase waiting on it: nothing of
the system is built from it, and nothing of the checks descends into it
beyond the host tests and the coverage gate every host crate has.

Tests: catalog 6.6.53 and 6.6.58.

## 8.22 Capacity for parallel work

- At most one side track besides the cryptography track is active at a
  time (D-45).
- A side track is worked on between phases, never instead of one.
- Order: track E, then the cryptography track to T7, then track D, with
  step D6 not started beside an XL phase; track F when a driver becomes
  foreseeable; G2 before Phase 3.
- Phase work whose logic passes the admission test may be pulled
  forward without changing its phase, its catalog items, or its
  acceptance criteria: `gfx` (Phase 9), `driver-i8042` (Phase 10), the
  QMP client and PPM reader (Phase 9), the allocator logic of `user-rt`
  and the encodings of `user-proto` (Phase 7). Section 12.9 lists them.
