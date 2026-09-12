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
| 12 | Time, randomness, and message interrupts | L | a userland thread reads a clock, waits until a deadline, draws entropy, and receives an MSI-X vector |
| 13 | PCI and the bus | M | a userland program enumerates the PCI bus and reports the virtio-net device and its registers |
| 14 | The network on the machine | XL | the system leases an address, resolves a name, and completes an HTTP request over a real device |
| 15 | TLS over the network | M | an HTTPS request from a program of the archive, with the certificate path validated |

Every phase has the same definition of done: all catalog items for the
components in the phase have tests, `sh tools/xtask-check.sh` is green, the
design documents reflect the code, the changelog is updated. The
[implementation plan](10-implementation-plan.md) specifies the work of
each phase down to crates, types, algorithms, and tests.

Beside the phases run tracks that depend on none of them: the
cryptography and TLS crates of section 8.21, specified in
[document 11](11-cryptography-and-tls.md), the tracks of sections 8.22
to 8.25, specified in [document 12](12-parallel-work.md), and the Secure
Shell client of section 8.26, specified in
[document 14](14-secure-shell-as-a-client.md). Section 8.27 states how
many of them may be active at once and which phase work may be pulled
forward. Of the tracks of documents 11 and 12 everything but the two
integration steps is finished, and those two are Phases 14 and 15; what
the four phases from 12 on need beyond them is specified in
[document 13](13-the-network-on-the-machine.md). Track S is decided and
not started (D-123).

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

Status: implemented.

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

Status: implemented.

Deliverables: `app-canvas` with a full-screen surface, a cursor that
follows the pointer, drawing while a button is held, and typed text
rendered with the bitmap font; the display server draws the cursor;
modifier handling and layout mapping in the input client library;
end-to-end tests that combine injected input with screendumps.

Tests: catalog 6.6.29 (combined items).

Acceptance: `sh tools/xtask.sh run --display` shows the canvas, which it
takes the screen for at the first event — the first movement of the mouse
in an interactive run; `sh tools/xtask.sh test --e2e` verifies cursor
movement, a drawn stroke, and typed text in screendumps.

Done: two programs cannot both hold a screen the size of the screen and
both be looked at, so the canvas presents nothing until an event reaches
it (D-126). That leaves the picture of `app-paint` standing to be checked
while the machine runs, and makes which of the two is on the screen a
matter of what happened rather than of which started last.

## 8.14 Phase 12: Time, randomness, and message interrupts

Three capabilities the kernel does not have and that everything above it
wants — time, randomness, and message interrupts. None of them is about
networking; all three are what
[document 13](13-the-network-on-the-machine.md) has to have before a
driver can be written.

Deliverables: `clock_now`, which answers with the microseconds since the
kernel started, computed from the tick count it already keeps and the
frequency it already reports, at tick resolution and documented as such;
`notification_wait_until`, which is `notification_wait` with a deadline
and answers zero bits when the deadline passed first, the existing call
left as it is; in `kernel-sched` a deadline on the `BlockedNotification`
state — a plain word of microseconds, so that no kernel crate has to
depend on `audhsos-time` — and one list ordered by that deadline, which
the tick handler walks from the front and stops at the first that has not
passed; `IndexList` of `audhsos-collections` gains the `insert_after` it
needs for that list and that it does not have today, which is the only
change this phase makes to a finished crate; `random_bytes`, which answers
with the four words a `ChaChaRng` seed is, each drawn from `RDSEED` with a
bounded number of retries and `Unavailable` rather than a word the
hardware did not give;
`interrupt_create_msi` on `SystemControl`, which allocates one vector out
of the same space the lines are allocated from and answers with the
`Interrupt` handle, the message address, and the message data, so that a
driver can program a device's MSI-X table itself; `interrupt_bind` and
`interrupt_ack` unchanged, with `interrupt_ack` on an MSI interrupt
clearing the outstanding flag and touching no hardware; two error codes
the table does not have, `Unavailable` for a source that would not deliver
and `NoVector` for a vector space with nothing left; the reference machine
gains `+rdrand,+rdseed` on its CPU model, as it gained its QMP socket in
Phase 9.

Tests: catalog 6.6.59, 6.6.60, and the `insert_after` item of 6.6.41.

Acceptance: a user program reads the clock twice around a wait of fifty
milliseconds and the difference is that wait to within a tick; a second
program waits on a notification nothing signals and returns at its
deadline; a third draws two seeds and they differ; an MSI vector created
by the root task and raised by a test device arrives as a signalled bit.

Done: the deadline list is threaded through the thread entries rather than
held as an `IndexList` over them, because a `Link` in no list is not zero
and the `Scheduler` lives in the one `static` that carries the object
pools (D-131). `IndexList::insert_after` is built and tested all the same;
it was the operation the type was missing. The vector space of a message
interrupt needed a gate in the interrupt descriptor table for every one of
its vectors before a device could write one: nothing routes a message, so
a vector without a gate arrives as a general protection fault.

## 8.15 Phase 13: PCI and the bus

Deliverables: `kernel-acpi` gains `mcfg.rs`, which reads the `MCFG` table
the way `madt.rs` reads the MADT — signature, length and checksum first,
then the allocation structures with their base address, segment group and
bus range; `system_info` reports the first allocation as four further
result words and four zero words on a machine whose firmware published no
`MCFG`; `Platform::ecam()` and its double; the kernel records the ECAM
range beside the `MmioReserved` regions, so that `memory_create_device`
admits the window whether or not the firmware's memory map marked it;
`boot::run` reports the window or its absence; the crate `pci` with the
`ConfigSpace` trait, the ECAM address arithmetic as a pure function, the
type-0 header, the enumeration bounded by the bus range, base address
register decoding with size probing that clears and restores the memory
decode bit in one call, the capability list bounded against a loop, the
MSI-X capability, and the vendor-specific capabilities of virtio 1.x from
section 4.1.4 of the OASIS specification; a volatile accessor over a
mapped region in `user-sys-x86_64`, checked against the region's length,
so that everything above it keeps `forbid(unsafe_code)`;
`docs/pcisig/README.md` and the provenance rule that takes the place of a
specification the repository may not hold; the reference machine gains
`-netdev user` with a forwarded port and
`-device virtio-net-pci,disable-legacy=on,mq=off`, because the proof that
the bus works is finding the device the next phase will drive.

Tests: catalog 6.6.61, and the fuzz targets `pci_config` and `mcfg`.

Acceptance: a program of the archive enumerates the bus and reports the
virtio-net device with its vendor and device id, the base address
registers it decoded, the four virtio capabilities it found, and the size
of its MSI-X table; on a machine started without the two network lines it
reports the rest of the bus, finds no virtio device, and ends by itself.

Done: the window of the reference machine covers all two hundred and
fifty-six buses, which is two hundred and fifty-six mebibytes, so
`app-lspci` maps one bus at a time at one address and takes it back before
the next: what the program costs is the page tables of one mebibyte,
whatever the firmware published. The ECAM range is recorded beside the
`MmioReserved` apertures whether or not the memory map marked it, and on
the machine this was built on it does not: the firmware of the reference
machine publishes the window in the `MCFG` table at `0xE000_0000` and
leaves it out of the memory map, so without that entry
`memory_create_device` would refuse the one aperture that makes the bus
reachable. The recorded configuration space of the `pci` double was read
out of the ECAM window of a running machine through the monitor, which is
the only part of a byte dump a probe of a base address register cannot
carry: the size masks the machine reported are recorded beside the
bytes.

## 8.16 Phase 14: The network on the machine

Deliverables: `driver-virtio-net` over a register trait with a scripted
double, negotiating `VIRTIO_F_VERSION_1` and `VIRTIO_NET_F_MAC` and
refusing every other offered bit by name — mergeable receive buffers, the
control queue, multiqueue and every offload among them — with the
initialization sequence over the state machine of `virtio-queue`, the
twelve-byte header of virtio 1.x, a receive path that returns every
buffer to the available ring in the call that took it, and a transmit
path that drains completions before it sends; one `Ram` memory object
with the `INFO` right as the driver's DMA region, granted whole by the
root task, holding both queues' rings and the frame buffers, whose
physical address `memory_info` answers; `server-net` around
`net-stack`, driving `poll` with the frames the driver hands it and
sleeping until `poll_at` on the notification that carries the MSI-X
vector and its clients; the socket protocol in `user-proto` with one ring
per socket in a shared memory object, as the input protocol of Phase 10
has one; `server-init` creates the ECAM device object, the DMA object and
the MSI vector and grants them; a client program that uses the protocol.
The reference machine needs nothing further: Phase 13 already put the
device on it.

Tests: catalog 6.6.62, 6.6.63, 6.6.64, and the fuzz target
`virtio_net_rx`.

Acceptance: `sh tools/xtask.sh test --e2e` verifies, in one run of the
whole system, that the driver reports the MAC address the command line
gave the device, that DHCP reaches a lease, that ARP resolves the
gateway, that a DNS query is answered, that a TCP connection through the
forwarded port carries a payload both ways and closes cleanly, and that
an HTTP `GET` over it returns a response the client parses; and then runs
the same image without the two network lines, where the server reports no
interface and the run ends by itself.

## 8.17 Phase 15: TLS over the network

This is step T8 of [document 11](11-cryptography-and-tls.md), which has
waited for a transport since the client was finished.

The wall clock this phase needed arrived ahead of it and is done (D-137,
catalog 6.6.71): the loader reads `GetTime` before it leaves the boot
services, the moment travels in the boot information, and `clock_wall`
answers the microseconds since the epoch with the source the firmware
named. Certificate validation had a `now` parameter and no value to put
in it; now it has one.

Deliverables: the transport glue that joins `audhsos-tls` to a TCP
connection of `server-net` — the record layer's bytes in and out of the
socket's ring, the handshake driven to completion against a deadline of
the clock of Phase 12, and the close notify in both directions; the
certificate path validated against the trust anchors the image carries,
against the date `clock_wall` answers; `tools/tls-probe` keeps its host
role and gains a counterpart that runs on the target.

Tests: catalog 6.6.65, with 6.6.71 already in.

Acceptance: an HTTPS `GET` from a program of the archive against a server
the test starts on the development machine, with a chain the test
certificate builder of `audhsos-x509` wrote, returns a response the
client parses; a chain with an expired certificate, one with a name that
does not match, and one signed by an anchor the image does not carry are
each refused with the alert the standard names.

## 8.18 Later work, not scheduled

a file system server on top of the FAT32, partition table and block
device logic of 8.24, which Phase 13 brings within reach because the bus
it needs is the one PCI gives it: what is left is the DMA region, the
process around the driver, and the protocol its clients speak; certificate revocation checking; virtio-gpu;
virtio-input or `usb-tablet` for absolute pointer coordinates; a
compositor with several windows; the `aarch64` port under HVF without a
loader; SMP with per-CPU run queues; hardware port permission bitmaps;
kernel-object memory donation; an interface definition language for
protocols; recursive capability revocation; a tickless timer; long file
names in the disk image writer.

## 8.19 Risks

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
| The shared foundations of 8.23 arrive after their consumers | the same containers and time arithmetic are written twice | track E is scheduled before the tracks and phases that need it, and is small |
| Phase 14 is XL and the network stalls in it | the release slips while three crates are half-finished | the driver, the server, and the protocol are separate crates with separate catalog items; the driver and the crate `pci` are logic over a trait and can be finished before the phase that integrates them |
| The kernel grows a deadline queue in the tick handler | every interrupt costs more | the list is ordered by instant, the walk stops at the first deadline that has not passed, and its length is bounded by the thread count |
| MSI-X cannot be masked by the kernel | a device that raises interrupts faster than its driver services them keeps a core busy | the driver suppresses through the used ring flag `virtio-queue` implements; the limit is written down in 13.5 rather than discovered |
| PCI-SIG specifications cannot be obtained and so are not kept beside the code | a layout constant is wrong and D-59's check does not exist for it | every constant names its document and revision; a configuration space captured from a real machine is a fixture of the crate's tests (D-124) |

## 8.20 Resolved decisions

The two questions that were open before Phase 0 are decided in the
decision register: the project name and crate prefix (D-34) and the build
entry point on the development machine (D-35, amended by D-64, which puts
the wrapper scripts of `tools/` in front of the proxy). No open decisions
remain.

## 8.21 Track C: cryptography and TLS

Status: specified in [document 11](11-cryptography-and-tls.md); steps T1
to T7 and R1 to R6 are implemented and reviewed as a whole. T8 is the
integration and is Phase 15: it needs a transport from track D, the
`random_bytes` system call, and the driver and server that carry the
bytes, and all three of those are Phases 12 to 14. What the track is
still waiting on, and who owns each piece, is section 11.14; what has to
exist under it is [document 13](13-the-network-on-the-machine.md).

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
| T8 | integration | M | Phase 15: the transport over a TCP connection of `server-net`, the `random_bytes` system call of Phase 12, and the HTTP client that `net-http` already is |
| R1 | `crypto-bignum` | M-L | implemented: the limb arithmetic moved out of `crypto-ec`, with a modulus known at run time and Montgomery exponentiation in a narrow and a wide form; a third form, the constant-time ladder for a secret exponent, came later with `crypto-dh` and belongs to no step of this track (D-122) |
| R2 | `crypto-rsa` | M | implemented: the key with its bounds, and PKCS #1 v1.5 verified by construction (D-80) |
| R3 | `crypto-rsa` | M | implemented: MGF1 and EMSA-PSS-VERIFY |
| R4 | `audhsos-x509` | L | implemented: the RSA identifiers with the NULL parameter rule of RFC 4055, the key, and the test certificates |
| R5 | `audhsos-tls` | M | implemented: the six code points under the `CertificateVerify` rule of D-82, and the RFC 8448 signature verified |
| R6 | fuzzing and the probe | S-M | implemented: the `rsa` fuzz target, and three RSA-rooted hosts reached by `tools/tls-probe` |

Definition of done per step, as for every phase: the catalog items of
6.6.30 to 6.6.38 and 6.6.55 that belong to the step have tests,
`sh tools/xtask-check.sh` is green, the documents reflect the code, the
changelog is updated.

## 8.22 Track D: the network stack

Status: D1 to D9 implemented, which is every step but the integration.
D10 is Phase 14, specified in
[document 13](13-the-network-on-the-machine.md).

Sans-I/O logic crates that consume and produce frames, take time and
randomness as parameters, allocate nothing, and depend on no kernel,
loader, or userland crate. The driver and the server that carry their
bytes are Phase 14.

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
| D10 | integration | XL | Phase 14 and Phase 15: the virtio-net driver, the network server, the socket protocol, and the `random_bytes` system call, and then the TLS transport jointly with T8 of 8.21; what has to exist under all of it is [document 13](13-the-network-on-the-machine.md) |

The stack carries IPv4 and IPv6 together (D-69), which supersedes the
first clause of D-50. An address is an `IpAddr` above `net-wire`, so the
transports, the resolver, and the facade are written once; what the second
family costs is the header format of D4 and its own address
configuration, not a second copy of everything above it.

Tests: catalog 6.6.42 to 6.6.50 and 6.6.54. Fuzz targets `ipv4`, `ipv6`,
`tcp_segment`, `dns_message`, `http_response`.

## 8.23 Track E: shared foundations

Status: implemented.

| Step | Crate | Size | Ends with |
|------|-------|------|-----------|
| E1 | `audhsos-time` | S | implemented: `UnixTime`, `CivilTime`, `Instant`, `Duration`, and the integer calendar of the proleptic Gregorian rule over the years 0 to 9999 |
| E2 | `audhsos-encoding` | S | implemented: strict Base64, hex, and PEM without allocation, with the fuzz target `pem` |
| E3 | `audhsos-collections` | M | implemented: `ArrayVec`, `RingBuffer`, `BitSet`, `IndexList` with `Link`, and `IndexMap`, each against a reference model |

Track E was scheduled first among the side tracks: step T5 of 8.21 needed
`UnixTime`, T6 needed PEM, track D needs all three, and phases 5 and 6
need `IndexList`. Track D is therefore unblocked from D1.

Tests: catalog 6.6.39 to 6.6.41. Fuzz target `pem`.

## 8.24 Track F: device logic without devices

Status: implemented.

| Step | Crate | Size | Ends with |
|------|-------|------|-----------|
| F1 | `virtio-queue` | M | implemented: the descriptor table, the two rings and the chain arithmetic over a memory access trait, with the free set in the queue's own memory rather than in the table the device can see; the initialization state machine with its two failure paths; no packed ring, no indirect descriptor and no `EVENT_IDX`, each refused by name at negotiation (D-52, D-99) |
| F2 | `fs-fat` | M | implemented: the boot parameter block against FAT12 and FAT16, cluster chains with every walk bounded by the cluster count, allocation and release with the free count counted at mount, directories in 8.3 form, and file read and write over a cursor; the xtask image writer is a user of it and keeps no FAT32 structure of its own (D-53, D-107) |
| F3 | `fs-gpt` | M | implemented: the protective record, both headers with the checksum rule of UEFI 2.11, table 5.5, the entry array and the walk of it, reading with the fallback to the backup the format prescribes, and writing in the order a torn write survives; the CRC-32 of the format lives here and the xtask keeps no partition table structure of its own (D-138) |
| F4 | `driver-virtio-blk` | L | implemented: the common configuration of virtio 4.1.4.3 and the register trait it is reached through, the feature set with every bit of virtio 5.2.3 named and all but three refused, the initialization of virtio 3.1.1 over the state machine of F1, and the request framing of virtio 5.2.6 with every rule of 5.2.6.1 refused at the call (D-139) |

Tests: catalog 6.6.51, 6.6.52, 6.6.72 and 6.6.73.

## 8.25 Track G: tooling

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

## 8.26 Track S: Secure Shell as a client

Status: decided in D-123, specified in
[document 14](14-secure-shell-as-a-client.md), begun. Steps S1, S2 and S3
are built: the wire types and the binary packet, the greeting and the
negotiation, both key exchange methods over `crypto-dh` (D-122) and
`crypto-ec::x25519` with the exchange hash and the six keys, and the
cipher over the packet layer. What is left needs the two decisions of
14.13: the host key of S4 and the authentication of S5.

The track is a client for SSH-2 and not a server, for the reason D-123
gives. It offers `curve25519-sha256` and `diffie-hellman-group14-sha256`
for the key exchange, `ssh-ed25519` for the host key and for `publickey`
authentication, and `chacha20-poly1305@openssh.com` as the cipher, and
needs no cryptographic primitive that the crypto track did not already
build. What it refuses, and why each name is refused, is section 14.5.

| Step | What | Size | Ends with |
|------|------|------|-----------|
| S1 | `audhsos-ssh`: `wire`, `packet` | M | implemented: the types of RFC 4251, section 5, against the vectors of that section, and the binary packet with its padding and its sequence numbers (catalog 6.6.68) |
| S2 | `kex` | L | implemented: the greeting, the message numbers, `SSH_MSG_KEXINIT` and the negotiation rule (catalog 6.6.69); both methods, the exchange hash, the six keys of RFC 4253, section 7.2, `SSH_MSG_NEWKEYS` and the aborts (catalog 6.6.70) |
| S3 | the cipher | M | implemented: `chacha20-poly1305@openssh.com` over the packet layer, against the worked example of the draft D-134 keeps in `docs/openssh/` (catalog 6.6.70) |
| S4 | host keys | S-M | the `ssh-ed25519` blobs of RFC 8709, the signature over the exchange hash verified, and the trust rule as a parameter |
| S5 | `auth` | M | `publickey` with the signature of RFC 4252, section 7, and `ext-info-c` with `server-sig-algs` |
| S6 | `channel` | L | channels, the window, the session channel, `exec` and `shell`, extended data, and `exit-status` |
| S7 | re-exchange | S-M | a re-exchange from either side, its two thresholds, and the disconnect reason codes of RFC 4250 |
| S8 | integration | M | the client over a socket of `server-net`, a program in the boot archive, and a handshake against a live OpenSSH; needs Phase 14 |

S1 to S7 depend on no phase and are built between them, as the whole of
track C was. S8 needs the network on the machine.

Tests: catalog 6.6.66, 6.6.68, 6.6.69 and 6.6.70 are written, for the
arithmetic of S2 and the whole of S1, S2 and S3; the rest are written
with the step that owns each. There is no RFC 8448 for this protocol — no document publishes a
complete handshake with the keys that made it — so the check from outside
is the interop test of S8 and not a replay, which is the one way this
track differs in kind from track C.

## 8.27 Capacity for parallel work

- At most one side track besides the cryptography track is active at a
  time (D-45).
- A side track is worked on between phases, never instead of one.
- Order: track E, then the cryptography track to T7, then track D, with
  step D6 not started beside an XL phase; track F when a driver becomes
  foreseeable; G2 before Phase 3.
- Track S (8.26) is the one side track that is not finished. The first
  rule covers it, and where it falls in the order above is not settled:
  the order is the history of the tracks that are done, and no phase
  requires track S of anything.
- Phase work whose logic passes the admission test may be pulled
  forward without changing its phase, its catalog items, or its
  acceptance criteria: `gfx` (Phase 9), `driver-i8042` (Phase 10), the
  QMP client and PPM reader (Phase 9), the allocator logic of `user-rt`
  and the encodings of `user-proto` (Phase 7). Section 12.9 lists them.
  Phases 12 to 15 add two more of that kind, and section 13.14 marks
  them: the crate `pci` of step N5 and the crate `driver-virtio-net` of
  step N7 are logic over a trait with a double and need no kernel, so
  either may be written before the phase that integrates it. Every other
  step of those phases changes the kernel, the reference machine, or the
  root task and is therefore phase work throughout.
