# Changelog

All notable changes to this project are documented in this file. The format
follows Keep a Changelog; the project follows Semantic Versioning.

## [Unreleased]

### Added

- `crypto-ec` gains ECDSA over P-384, which is what a chain that ends at a
  P-384 root takes. `p384::PublicKey::from_sec1` reads the uncompressed
  point of ninety-seven bytes, `verify` checks a signature against a
  digest of any width up to the order, and `sign` behind `test-signing` is
  deterministic per RFC 6979 over SHA-384. The constants are RFC 5903,
  section 3.2, and RFC 5114, section 2.7, states them independently and
  agrees. The vectors are RFC 6979, appendix A.2.6 — the ten signatures of
  two messages under five hashes — and RFC 5903, appendix 8.2, whose two
  key pairs and shared point are three scalar multiplications the tests
  did not compute.
- `crypto-ec` grows two modules the two ECDSA curves share:
  `montgomery` is the modular arithmetic, generic over the number of
  limbs, and `jacobian` is the group law of a short Weierstrass curve
  whose `a` is minus three. `p256` and `p384` are now the constants and
  the ECDSA on top of them. P-256 is unchanged in behaviour: the same RFC
  6979 vectors pass over the shared code. An element carries the width of
  its encoding as a parameter, and a width that does not match its limbs
  fails to compile.
- `audhsos-x509` reads and verifies P-384 keys: `oid::SECP384R1`,
  `SubjectPublicKey::EcdsaP384`, and the pairings of that key with
  `ecdsa-with-SHA256` and `ecdsa-with-SHA384`. The curve named in the
  algorithm and the width of the point must agree, so a P-384 identifier
  over a P-256 point is a bad key rather than either curve. The test
  certificate builder gains `TestKey::EcdsaP384Sha384`, so a whole chain
  of that curve is one the test suites build rather than vendor; the TLS
  client is driven through a handshake over such a chain.
- `audhsos-x509`: `TrustAnchor::from_certificate` reads the subject and the
  key out of a certificate and stops there. The certificate's own
  signature is never verified, which is what lets a root that reaches a
  system only as a cross-signed certificate serve as an anchor: the
  `GTS Root R4` in Google's chain is signed by GlobalSign with RSA, and
  `Certificate::parse` refused it for an algorithm nothing would have
  looked at. The key is checked for being one this crate can verify with,
  so an unusable anchor is refused while the caller still holds the file
  rather than becoming a path that reaches nothing. Decision D-62.
- `tools/tls-probe`: a host program that drives the sans-I/O client over a
  real socket, so that the stack is answered by a server instead of by a
  recording. It opens TCP, runs the handshake, and speaks enough HTTP/1.1
  to show a status line and a body. Everything above the socket is this
  repository's code; the host supplies the socket, the wall clock, and
  `/dev/urandom` under `crypto-rng`. It is a workspace of its own, like
  `fuzz/`, because it is the only crate in the tree that links `std`. Its
  anchor is a pinned intermediate rather than a root: a chain to a P-384
  root cannot be walked to the end until `crypto-ec` has that curve, which
  the README of the probe writes down as the one thing the run does not
  prove.
- `kernel-acpi` (Phase 4): the ACPI tables the kernel needs to find its
  interrupt controllers, parsed in safe Rust. `parse_rsdp` reads the root
  pointer of revision zero or two with both of its checksums;
  `SdtHeader::parse` reads and checks the header every table starts with;
  `RootTable` walks the RSDT or the XSDT, four-byte entries or eight-byte
  ones as the signature says; `madt::parse` reads the multiple APIC
  description table into fixed capacities of four I/O APICs and sixteen
  interrupt source overrides. An entry of length zero is an error, because
  a walk that accepted one would never end; an entry that leaves the table
  is an error; more I/O APICs than the kernel holds are an error and not a
  truncation; an entry of a type the parser does not read is skipped by
  its length.
- `kernel-acpi`: `Madt::route_isa` answers where an ISA line goes and how
  it is taken, which is the one piece of interrupt routing that is logic
  rather than register writes, and is therefore tested on the host.
- `kernel-x86-tables` gains the register blocks of the two APICs and the
  encoding of a redirection entry, the write sequence that moves the two
  legacy controllers out of the way and masks them, and the vector plan:
  exceptions `0..=31`, legacy controllers `0x20..=0x2F`, timer `0x30`,
  I/O APIC lines `0x40 + gsi`, system call `0x80`, spurious `0xFF`.
  `kernel-hal-x86_64::vectors` is that module, so that the plan is one
  table with host tests behind it.
- `kernel-hal-x86_64::acpi` finds the tables of the machine through the
  physical window and hands their bytes to the parsers. No range is read
  before it has been checked against the memory the firmware reported, so
  a root pointer that names nothing makes the kernel report rather than
  fault.
- `kernel-hal-x86_64::apic`: `LocalApic` and `IoApic` over their register
  windows, one volatile access per `unsafe` block, and `Apics`, which
  implements both `InterruptController` and `Timer`. A line is routed
  masked, and the routing resolves ISA lines through the overrides of the
  table. `Apics::line_state` reads a redirection entry back out of the
  hardware, so a test can assert what the I/O APIC took rather than what
  the kernel meant to write.
- `kernel-hal-x86_64::timer`: the local APIC timer measured once against
  channel two of the interval timer, ten milliseconds with a bounded poll,
  then programmed periodic at `TICKS_PER_SECOND`. Every poll is bounded, so
  a machine whose channel two does not run reports instead of hanging.
- `kernel-hal-x86_64::interrupts`: the bring-up that reads the tables, maps
  the two register windows through a mapping the kernel supplies, quiets
  the legacy controllers, turns the local APIC on, and masks every I/O APIC
  line. `acknowledge` sends the end-of-interrupt, and sends none for the
  spurious vector, which is the one that expects none.
- `kernel-hal-x86_64::instructions`: `rdmsr` and `wrmsr`, and
  `InterruptGuard`, which turns interrupts off for as long as a borrow of
  kernel state lasts and back on afterwards if they were on. This is the
  guard the safety policy names in 4.6.
- `kernel-hal-x86_64::traps` gains a handler for every device vector of the
  plan and a second registration point, `set_interrupt_handler`: a device
  interrupt carries its vector and nothing else, which is not what a trap
  report carries.
- `kernel-hal-x86_64::testing::raise_interrupt` raises a vector from
  software, with the vector as an inline constant. The interrupt tests
  therefore assert what they mean to assert instead of asserting that a
  handler is installed.
- `kernel-core::tick` counts a timer tick and gives the scheduler its turn,
  which is nothing until Phase 5. There are two tick counts: the adapter
  counts what the hardware delivered, because `Timer::ticks` is the
  adapter's method and the adapter may not depend on `kernel-core`, and
  `KernelState` counts what the kernel processed. `KernelState` gains the counts of ticks,
  of device interrupts, and of spurious interrupts, and `with_state` hands
  the state out the way `with_memory` hands out the memory.
- `kernel-core::memory`: `KernelMemory::map_device` maps a device register
  window into the physical window, uncached, and registers it as a region
  of kind `Device`. The window the loader builds covers memory, because it
  is sized from the memory map; an aperture above it is the kernel's own to
  map, at the address the window rule gives it.
- The kernel image brings the interrupt hardware up, starts the timer, and
  waits for its first ticks before it reports that the boot is complete.
- QEMU test image `interrupts`: the tables name the hardware and the unit
  is on; a second tick arrives after the end-of-interrupt; the tick counter
  grows while the kernel does nothing; a masked timer delivers nothing and
  unmasking starts it again; a vector raised from software reaches the
  handler of that vector, for `0x30`, `0x40`, and `0xFF`; a routed line
  carries the vector and the wiring the table names, comes up masked,
  follows `mask` and `unmask`, and refuses a second routing. Catalog
  6.6.11, the APIC items of 6.6.16, and the interrupt items of 6.6.21.
- Fuzz target `madt` over the ACPI parsers, with fifteen seeds. The target
  reads the bytes twice: as they are, so that signature, length, and
  checksum are exercised, and once with those three repaired, so that the
  fuzzer reaches the walk over the entries without having to guess a
  checksum.

- `audhsos-collections` (track E3): `ArrayVec`, `RingBuffer`, `BitSet`,
  `IndexList` with its `Link`, and `IndexMap`. Fixed capacity, no
  allocation, no `unsafe`, and no panic: a `push` that does not fit
  returns `Err(Full)` and every accessor returns an `Option`, so nothing
  here can abort a system call or a packet. Track E is complete, and track
  D is unblocked from D1.
- `audhsos-collections`: `IndexList` holds no values. It holds a head, a
  tail, a length, and an identifier, while the links live in a slice of
  `Link` the caller keeps beside its own array — which is what a run queue
  over a fixed array of threads is, and what an endpoint wait queue over
  the same array is (D-48). Several lists may run over one slice, one per
  priority, and a `Link` records which list its node is in, so a node
  handed to the wrong list is refused instead of being stolen from the
  right one.
- `audhsos-collections`: `BitSet` is parameterised by its number of 64-bit
  words rather than by its number of bits, because `[u64; BITS.div_ceil(64)]`
  is an expression over a const parameter and stable Rust cannot size an
  array with one. The alternative was an incomplete language feature in
  the crate every other crate rests on. `BitSet::BITS` reports the size and
  every operation is checked against it.
- `audhsos-collections`: the owning containers store `Option<T>`, which
  costs one discriminant per slot and buys the right to hold a `T` with no
  default without a line of `unsafe`. The price is that there is no
  `as_slice`, and it is stated in the crate documentation rather than left
  to be discovered.
- Catalog 6.6.41 gains the further items of `IndexList` and `BitSet`, the
  requirement that every owning container carry a value that is neither
  `Copy` nor `Default`, and the note that the model generators reach
  beyond the container as well as inside it.

- `audhsos-encoding` (track E2): strict Base64 of RFC 4648, hex, and PEM of
  RFC 7468, each writing into a buffer the caller owns and answering how
  many bytes it wrote. Nothing allocates and nothing panics. Strict means
  one sequence of bytes has one text: Base64 refuses whitespace, a missing
  or excessive pad, a pad anywhere but at the end, and a final quantum
  whose unused bits are not zero.
- `audhsos-encoding`: PEM in the strict form. The end line must name the
  label of the begin line, every body line but the last is exactly 64
  characters, only the last may carry a pad, and nothing but line
  terminators may follow the end line. Explanatory text before the begin
  line is skipped, which RFC 7468 permits and a certificate file with a
  preamble needs. A block decodes into two borrows: the label into the
  input, the bytes into the caller's buffer.
- `audhsos-encoding`: a text with no end line reports the missing line and
  not the length of a body line. The body is located before it is read, so
  the rule for the last body line is applied only to a block that has one.
- `fuzz/pem`: the target track E owed, with a corpus of twelve texts — a
  block, a preamble, `CRLF` terminators, and the eight shapes that are
  refused. It asserts what strictness means: an accepted block re-encodes
  to a text that decodes to the same bytes, and a Base64 text the decoder
  accepts re-encodes to exactly itself.
- Catalog 6.6.40 gains the two non-canonical quanta by name, the split
  between a length error and a character error, the PEM rules that were
  not in the first list, and the check that the generator of near-valid
  blocks reaches both an accepted and a refused text.

- `audhsos-time` (track E1): `CivilTime`, `UnixTime`, `Instant`, and
  `Duration`, and the integer calendar that joins the first two. The rule
  is the proleptic Gregorian one over the years 0 to 9999, which is the
  range a `GeneralizedTime` can write down; the arithmetic is March-based,
  so a year ends with its leap day and the day of the year needs no month
  table. Every intermediate value is bounded by the year range the module
  validates before it computes, and a result outside the range is an error
  rather than a wrap.
- `audhsos-time`: the crate reads no clock. Time enters every interface as
  a parameter, which is what lets a sixty-second backoff be exercised in
  microseconds of wall clock. An `Instant` counts microseconds from an
  origin the caller chooses, and its `Add` saturates, because a timer that
  saturates fires late while one that wraps fires immediately and forever.
- `audhsos-time`: generators of all four types behind the feature
  `test-strategies`, for track D and `fs-fat`.
- Catalog 6.6.39 gains the day-by-day walk of the calendar, the ends of the
  range, the resolution of a `UnixTime`, and the items of the generators.
  Every day from 1601-01-01 to 9999-12-31 round-trips and is the successor
  of the day before it.

- `audhsos-symbols` (track G2): an address to a function, a file, and a
  line. The symbol table gives the function, the DWARF line program of
  version 4 or 5 gives the file and the line. The state machine runs once
  per lookup and keeps only the row it needs, so the crate allocates
  nothing and borrows everything from the bytes it was handed. No inline
  frames and no call-frame information, and therefore no stack unwinding.
- `audhsos-elf`: the section header table, which the loader does not read
  and a symbolizer cannot do without. `sections()` needs the magic, the
  class, and the byte order and nothing about segments, so a file without
  a loadable segment still yields its sections.
- `audhsos-symbols`: `demangle` writes a Rust symbol name back readable,
  in the `v0` scheme of RFC 2603 and the legacy `_ZN` scheme, straight
  into a formatter and therefore without allocating. Generic arguments are
  dropped and a name the parser does not understand is written unchanged.
  Every one of the 1002 symbols of the kernel image reads back.
- `xtask`: `symbolize <elf> <address>...` answers by hand, and a QEMU run
  that fails now resolves every address of the kernel half in its serial
  output against the image it ran. A file that cannot be read or carries
  no symbols produces nothing, because the report is a comment on a run
  that already failed.
- Catalog 6.6.53 gains the section header table items, the forms and the
  odd opcodes of a line program, and the items of the xtask.

- `fuzz-support` (track G1): the `LLVMFuzzerTestOneInput` entry glue, the
  `fuzz_target!` macro that writes it once, and the corpus replay. The one
  `unsafe` of the project's fuzzing turns the fuzzer's pointer and length
  into a slice and answers the empty slice for a length of zero or a null
  pointer; Miri covers it.
- `fuzz/`: a workspace of its own with the targets whose parsers exist,
  `elf`, `boot_image_header`, and `boot_info`, each with a seed corpus
  under `fuzz/corpus/`. A target built with the coverage instrumentation
  and `--cfg fuzzing` is a libFuzzer binary; the same source built without
  them replays a corpus and needs no fuzzer runtime.
- `xtask`: the directories the checks never descend into are a table in
  `policy.rs`, where D-24 puts a policy, instead of a constant of the
  file walker. `research`, which holds source of other projects kept to be
  read, joins `target` and `.git`: that source carries the license headers
  of those projects and not this one, and a check of this project has no
  business in it.
- `xtask`: `fuzz --regression` replays the stored corpus of every target
  and is the tenth step of `check`. `fuzz` itself now passes the corpus
  directory to the fuzzer and uses the coverage instrumentation flags
  rather than `-Zsanitizer=fuzzer`, which rustc does not accept: the
  libFuzzer runtime comes from the platform's clang, and a machine without
  it fails at the link step.

- `audhsos-abi`: `KERNEL_STACKS_BASE`, `KERNEL_STACK_PAGES`,
  `KERNEL_STACK_SLOT_PAGES`, `KERNEL_STACK_SLOTS`, and
  `MAX_PHYS_WINDOW_BYTES`.
- `kernel-mm`: the kernel stack pool. A slot is one unmapped guard page
  followed by four mapped pages; an allocation that runs out of frames
  leaves no slot taken and no page mapped. `NoFrames`, a frame source for
  a walk that creates no table, and `Mapper::frames_mut`.
- `kernel-core`: `config`, the number of each kind of kernel object and
  the size of the fixed tables of the kernel address space; `memory`, the
  bring-up. It takes the reserve out of the normalized map in the size the
  boot image header asks for, adopts the loader's page tables by walking
  the kernel image, the physical window, the boot stack, and the boot
  information page into a kernel region table, drops the loader's identity
  mapping without giving a frame back, and stores the result in the
  `MEMORY` cell.
- `kernel-hal-x86_64`: `PhysicalWindow` with `frame_bytes_mut`, which
  replaces `WindowAccess`; `active_root` and `activate` beside `LocalTlb`;
  and `memory`, the walk of the active tables through the window that the
  bring-up and the boot information page need.
- `audhsos-kernel`: the kernel takes its memory over after the boot report
  and reports the reserve, the regions of its address space, and the
  identity mapping it dropped. Three test kernels, `memory`,
  `memory_fault`, and `kernel_stack`, cover the memory and kernel stack
  items of catalog 6.6.21.
- `kernel-core`: `KernelMemory::allocate_stack` and `release_stack`, which
  build the mapper out of the root frame and the reserve and drive the
  stack pool.
- `kernel-hal-x86_64`: `testing::write_byte`, the write a test image needs
  to show that a page it mapped carries what it wrote and that a guard
  page faults.
- Catalog 6.6.21 gains the kernel stack items and the boot information
  address item.
- Plan 10.5.0: what the kernel reserve carries and what the kernel image
  carries, with the arithmetic behind D-57.
- Planning documents and the decision register under `docs/`.
- `docs/11-cryptography-and-tls.md`: the design and implementation plan
  for the TLS 1.3 client track (constant-time primitives, hashes and
  HKDF, AEADs, elliptic curves, random generator, DER, X.509, the sans-I/O
  protocol crate), its test catalog entries 6.6.30 to 6.6.38, its roadmap
  track 8.17, and decisions D-36 to D-44.
- `docs/12-parallel-work.md`: the design and implementation plan for the
  work that runs beside the kernel phases — the admission test for
  parallel work, the sans-I/O network stack (`net-wire`, `net-eth`,
  `net-ip`, `net-udp`, `net-tcp`, `net-dns`, `net-dhcp`, `net-http`,
  `net-stack`), the shared foundations (`audhsos-time`,
  `audhsos-encoding`, `audhsos-collections`), the device logic without
  devices (`virtio-queue`, `fs-fat`), the tooling (`fuzz-support`,
  `audhsos-symbols`), the phase work that may be pulled forward, and the
  capacity rule; its test catalog entries 6.6.39 to 6.6.53, its roadmap
  tracks 8.18 to 8.22, and decisions D-45 to D-54.
- `crypto-ct`: `Choice`, constant-time comparison, selection, exchange and
  copy, `Secret<N>` with a best-effort erase on drop.
- `crypto-hash`: SHA-256, SHA-384, SHA-512, HMAC, and HKDF, against the
  vectors of FIPS 180-4, RFC 4231, and RFC 5869.
- `crypto-aead`: `ChaCha20`, `Poly1305`, and the `ChaCha20-Poly1305`
  authenticated cipher of RFC 8439, sealing and opening in place, with
  verification before decryption.
- `crypto-aead`: AES-128 and AES-256 bitsliced over four blocks without a
  lookup table, table-free GHASH, and AES-128-GCM and AES-256-GCM, against
  the published test cases of the mode.
- `crypto-ec`: the field of `2^255 - 19` and X25519 with a constant-time
  Montgomery ladder, against the vectors of RFC 7748.
- `crypto-ec`: Ed25519 verification and deterministic signing behind
  `test-signing`, with strict canonicality and small-order checks, against
  the vectors of RFC 8032.
- `crypto-ec`: P-256 with Montgomery arithmetic for both moduli, Jacobian
  point arithmetic, ECDSA verification, and deterministic signing per
  RFC 6979 behind `test-signing`.
- `crypto-rng`: the `Entropy` and `Rng` traits, a `ChaCha20` generator that
  rekeys after every request and mixes fresh material into its key when it
  reseeds, and the doubles the protocol tests will need.
- `audhsos-der`: a strict, zero-copy reader for the distinguished encoding
  rules, with bounded nesting and one encoding per value. Its time
  conversion waits on `audhsos-time` (D-46); section 11.14 of document 11
  lists that seam and the others.
- `audhsos-x509`: certificate parsing and signature verification, chain
  validation against caller-supplied trust anchors, and RFC 6125 name
  matching, with a builder behind `test-certificates` that writes and signs
  the certificates the tests use.
- `audhsos-tls`: the record layer, the record protection, the key schedule
  of RFC 8446 section 7.1, and the handshake transcript.
- `audhsos-tls`: the wire codec and the handshake messages, read against
  the server side of the trace of RFC 8448.
- `audhsos-tls`: the client state machine, the alerts, and the sans-I/O
  interface. The handshake of RFC 8448 is reproduced, and a whole
  connection runs against a server built in the tests.
- Fuzz targets `der`, `x509`, `tls_record`, and `tls_handshake`, the four
  that catalog 6.6.35 to 6.6.38 requires of track C. Beyond "no input may
  panic" each one carries an invariant: a DER value is shorter than what
  it was read from, a parsed certificate is a view of its input and
  verifies against no empty trust store, a record's length is its header
  and its body and what this crate seals it opens again, and no handshake
  reader hands back more than the message it was given. The corpora start
  from the trace of RFC 8448 and from certificates the builder writes.

- `docs/rfc/`: the standards this system implements, verbatim, with their
  source and checksum recorded (D-59). RFC 8448, whose trace is the test
  the TLS client must reproduce; and the four documents P-384 takes — RFC
  5903 for the curve parameters, RFC 5480 for the `secp384r1` identifier
  and the uncompressed point encoding, RFC 5758 for `ecdsa-with-SHA384`,
  and RFC 6979 appendix A.2.6 for the signature vectors, which is the
  table one curve up from the A.2.5 the P-256 tests already read. The
  index says what each document contributes, and which ones were read and
  left out.
- Decision D-56: the key exchange of the TLS client is `x25519` alone.
- Workspace foundation: pinned toolchain, workspace lint set, SPDX headers,
  license, CI workflow.
- `audhsos-abi`: error codes, rights, object types, handles, layout
  constants.
- `kernel-types`: physical and virtual addresses, frames, pages, ranges,
  alignment.
- `kernel-hal-api`: hardware abstraction traits with test doubles.
- `audhsos-sync`: the `Global<T>` cell for global state.
- `test-support`: property-test engine with integrated shrinking and
  model-test runner.
- Unit tests live in `src/tests/` so that coverage measures product code
  only.
- `xtask`: `lint`, `check-layering`, `check-deps`, `unsafe-budget`,
  `test`, `coverage`, `miri`, `doc`, `check`.
- `audhsos-abi`: boot image header and boot information structure with
  validating parsers and writers, and generators behind the feature
  `test-strategies`.
- `kernel-objects`: fixed-capacity object pool with generation-checked ids,
  reference counts, and first-in-first-out slot reuse; quotas.
- `kernel-mm`: memory map normalization, kernel reserve selection, bitmap
  frame allocator, `x86_64` page-table entries behind an
  architecture-neutral trait, the mapper over the HAL traits with bounded
  range operations, and the region table of an address space.
- `kernel-types`: `PhysFrame::ZERO` and `PhysFrameRange::EMPTY`.
- `kernel-hal-api`: `MemoryFrameAccess::with_lazy_tables`, which
  materializes a page table on the first modifying access to a frame of a
  declared memory range.
- `audhsos-elf`: validating ELF64 parser with an image builder behind the
  feature `test-strategies`.
- `driver-uart16550`: register logic of the 16550 serial controller with a
  recording register double behind the feature `test-doubles`.
- `kernel-x86-tables`: encoding and decoding of the global descriptor
  table, the interrupt descriptor table, and the task state segment.
- `audhsos-uefi`: layouts, status codes, and identifiers of the UEFI
  interfaces the loader uses, the memory map reader that honors the
  firmware's stride, the conversion into boot regions, and UTF-16
  encoding; the Graphics Output Protocol structures, `LocateProtocol`, and
  the conversion of a graphics mode into a framebuffer description.
- `audhsos-abi`: the boot information carries the framebuffer the firmware
  set up. The fixed part grows to 136 bytes; the version stays 1.
- `xtask`: the disk image writer with its own CRC-32, GUID partition
  table, and FAT32 file system, the boot image writer, and the `image`
  subcommand; the policy table names the target every crate is built for,
  and the host commands skip the crates that are not built for the host.
- `kernel-core`: the boot report, the trap report, and the cell holding the
  global kernel state.
- `kernel-test-harness`: the test runner of a kernel image and the serial
  line protocol it writes.
- `kernel-hal-api`: a mutable reference to a debug console or an exit
  device is one, so that the kernel can hand one out without giving it
  away.
- `kernel-hal-x86_64`: the first adapter crate. Privileged instruction
  wrappers, the descriptor tables, the trap handlers, the boot information
  as a `Platform`, the serial debug console, the exit device, and page
  table memory through the physical window.
- `audhsos-kernel`: the kernel image with its linker script, the entry the
  loader jumps to, and the panic handler.
- `audhsos-abi`: `BOOT_STACK_TOP`, `BOOT_STACK_PAGES`, and
  `BOOT_INFO_VADDR`.
- `xtask`: the `build` subcommand, and a check that the constants the
  linker scripts repeat agree with the ABI.
- `boot-uefi-x86_64`: the loader. It reads the kernel and the boot image
  from the boot volume, places the kernel image in one physical range,
  builds the physical memory window, an identity mapping, the kernel
  segments, the boot stack, and the boot information page with the
  kernel's own mapper, writes the boot information from the memory map it
  reads last, and enters the kernel.
- `xtask`: `qemu-runner`, `run`, and `test --qemu`. The runner wraps a
  test kernel into a disk image, runs the reference machine with a time
  limit, reads the serial protocol, and maps the exit status; `test
  --qemu` runs every test kernel and the three images the loader has to
  reject; `check` runs it last.
- `kernel-hal-x86_64`: `testing`, what a kernel test image needs: the
  harness over the debug console and the exit device, the `test_kernel!`
  macro that writes the entry point and the panic handler once, the trap
  hook a test image registers, and the instructions that raise the
  exceptions the trap tests expect.
- `audhsos-kernel`: ten test kernels under `tests/`: boot, console,
  descriptors, breakpoint, divide error, invalid opcode, general
  protection, page fault, the double fault of a kernel stack overflow, and
  a panic in an image that expects one.
- `audhsos-abi`: the boot stack is 64 pages, not 16. An unoptimized test
  image needs more than 64 KiB before it reaches the harness.
- CI installs QEMU and the UEFI firmware and names the firmware the
  distribution installed, so that `check` can run `test --qemu`.
- Planning for graphics output and input devices: roadmap Phases 9 to 11,
  decisions D-29 to D-33, catalog sections 6.6.24 to 6.6.29, and the
  framebuffer fields of the boot information structure in the documents.

### Changed

- `audhsos-der` no longer defines a time type. `Timestamp` is gone;
  `read_time`, `from_utc_time`, and `from_generalized_time` yield the
  `CivilTime` of `audhsos-time`, which closes the seam decision D-46
  opened. The parser kept the syntax — the form RFC 5280 allows, the
  digits, the `Z` suffix, the two-digit year window — and gave up the field
  ranges to the calendar. A day is now checked against the true length of
  its month, so the thirty-first of April and the twenty-ninth of February
  of a year that is not leap are refused where the old check against
  thirty-one let them through.
- `audhsos-x509` and `audhsos-tls` take the type from its owner:
  `Certificate::not_before` and `not_after`, the `now` of `verify_chain`,
  and the `now` of `ClientConfig` are a `CivilTime`. No signature changed
  shape, because the fields and their order did not.
- Section 11.14 loses the first of its four seams. What remains missing is
  not a conversion but a clock: no crate of this project reads one, so the
  value still enters from outside.

### Fixed

- `audhsos-tls`: a `Certificate` message was refused whole when any entry
  in it failed to parse. RFC 8446 section 4.4.2 makes the entries behind
  the leaf an aid to path building and allows ones that belong to no path,
  and a server that sends its own root sends a certificate this client
  cannot read: Google Trust Services puts a P-384 root above a P-256
  chain, and `crypto-ec` has P-256 and Ed25519. Every such server was
  unreachable. An entry that does not parse is now passed over. The leaf
  still has to parse, and the path still has to reach an anchor through
  signatures that verify, so nothing a path check decided has changed.
- `audhsos-tls`: the alert that ends a connection went out under the
  handshake keys, and those are dropped the moment the application keys
  exist, so after the handshake it was written in the clear and the server
  could not read why its peer had gone. It now uses the keys of the epoch
  the connection has reached.
- `audhsos-tls`: the `change_cipher_spec` record of middlebox
  compatibility mode was accepted anywhere and whatever it carried. RFC
  8446 section 5 closes its window with the peer's `Finished` and allows
  it one value, and both are enforced.
- `audhsos-tls`: the sequence number is spent before its nonce is used
  rather than after, so that no path can hand the same nonce out twice.
  The last record of an epoch is still written; every call after it is
  refused.

- Plan 10.4 said that raising an interrupt vector from software was "not
  possible without asm" and settled for a weaker assertion about the
  spurious vector. That was written before `kernel-hal-x86_64::testing`
  existed, which now holds four `asm!` sites of its own and is allowlisted
  for them. The section says what the test is to assert, carries the
  `int` with the vector as an inline constant that the pinned toolchain
  accepts, and 4.5 lists the site.

- `kernel-hal-x86_64`: the boot information page was reported as a memory
  region with its virtual address, which `PhysAddr::new` rejects, so the
  region was silently dropped and no boot report ever showed a
  `boot-info` line. The entry point now walks the loader's tables for
  `BOOT_INFO_VADDR` and reports the frame that walk names.

### Changed

- The object pools live in the `.bss` of the kernel image, not in the
  kernel reserve (D-57): a `Pool<T, N>` is a typed array, and putting one
  into raw frames would need `unsafe` in a logic crate. Document 2.4.1 said
  the reserve holds them and is corrected. The reserve holds page tables,
  kernel stacks, and the IPC buffers of threads.
- The handle slots of the machine are one shared arena of `HANDLE_ENTRIES`
  (16384) and `HANDLES_PER_PROCESS` (4096) is the ceiling the quota
  enforces against it, not memory set aside per process (D-58). This
  supersedes the handle part of D-57: 1024 handles were too few for
  `server-memory`, which holds one handle per memory object it hands out
  and can therefore reach `MEMORY_OBJECTS`, and 4096 inside every `Process`
  would have been 8 MiB of `.bss` for the one process that needs them.
  Plan 10.5.2 describes the arena, and 2.3.2 is worded for it.
- `THREADS` and `KERNEL_STACKS` are 256 and `KERNEL_STACK_SLOTS` follows
  them, because every thread costs five frames of the reserve and 1024 of
  them would need 5120 against the 4048 the reference machine has.
  `PROCESSES` is 64 and `HANDLES_PER_PROCESS` is 1024, because the handle
  table lives in the `Process` object and 256 tables of `1 << 16` entries
  would be 512 MiB of `.bss`. The reference machine keeps its 256 MiB.
- `kernel-hal-x86_64::WindowAccess` becomes `PhysicalWindow` in a module
  of its own and gains the byte access the IPC buffers and the boot image
  header need; `paging` keeps the lookaside buffer and the page-table
  root.
- The kernel binary carries `kernel-hal-api`, `kernel-mm`, and
  `kernel-types` as dev-dependencies for its test kernels; the kernel
  image itself keeps its three dependencies.
- `UnixTime` moves out of `audhsos-der` into the new `audhsos-time`
  crate, and the trust-anchor PEM decoding into `audhsos-encoding`
  (D-46, D-47); document 11 is amended accordingly.
- The crate catalog, the repository layout, the layering rules, and the
  duplication table of document 5 list the crates of document 12; the
  shared-crate list of layering rule 4 gains the three foundations.
- The roadmap and the document index state the status of track C as
  implemented through step T2, which the crates already were.
