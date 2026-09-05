# 12. Work Parallel to the Kernel Phases

Document 11 established a pattern: a set of crates that is pure logic,
host-testable, and free of any dependency on the kernel, the loader, or
userland can be built while the kernel phases run, because it competes
for nothing but attention. This document collects every other piece of
work with that property, specifies it, and orders it. It is both design
and implementation plan, in the form of documents 10 and 11.

## 12.1 Purpose

The kernel phases are a chain: each one needs the one before it. The
work in this document is not part of that chain. It exists so that the
system has something to run on top of the kernel when the kernel is
there, and so that the phases themselves get shorter, because the
containers, the time arithmetic, and the diagnostics they need already
exist when they start.

Nothing here changes the phase order. The roadmap remains binding.

## 12.2 The admission test

A candidate belongs in this document when it passes all four:

1. **Pure logic.** No I/O, no hardware access, no privileged
   instruction. Time, randomness, and transport bytes enter as
   parameters. The crate computes; someone else moves bytes.
2. **The lint profile of a logic crate.** `no_std`,
   `#![forbid(unsafe_code)]`, no allocation, every fallible operation
   returns `Result`, no panic path.
3. **No upward dependency.** It depends on layer-0 logic crates and on
   crates of its own track only. Never on a kernel, loader, or userland
   crate; those depend on it, not the reverse.
4. **A complete catalog entry can be written today.** If the edge cases
   cannot be enumerated and tested on the host before the kernel runs,
   the component is phase work.

A candidate that fails one of the four is phase work and stays in the
roadmap where it is.

## 12.3 What does not qualify

Servers, applications, HAL adapters, `user-sys-x86_64`, driver adapters,
the system call surface, the boot image assembly, and every end-to-end
test. Their logic is already cut so that it sits behind a trait, but
what remains in the phase is the integration, and integration needs the
kernel. Section 12.9 lists the parts of that phase work whose *logic*
passes the admission test and may therefore be written early; the
integration around it stays in its phase.

## 12.4 Inventory

| Track | Content | Size | Relation to the rest |
|-------|---------|------|----------------------|
| C | cryptography and TLS ([document 11](11-cryptography-and-tls.md)) | XL | in progress; its step T8 waits for track D |
| D | the network stack, sans-I/O (12.6) | XL | unblocks C's transport; unblocks HTTP |
| E | shared foundations: time, encodings, collections (12.5) | M | `audhsos-time` and `audhsos-encoding` implemented; needed by C at T5 and T6, by D throughout, by phases 5 and 6 |
| F | device logic without devices: virtqueues, FAT32 (12.7) | M | prepares the network and storage drivers that are later work |
| G | tooling: fuzz support, symbolization (12.8) | M | serves every track and every phase |

Track E comes first in this document because tracks C and D both rest on
it.

## 12.5 Track E: shared foundations

Three small layer-0 crates that several tracks need and that are today
either missing or about to be written twice.

### 12.5.1 `audhsos-time`

Implemented.

```rust
pub struct UnixTime(i64);   // seconds since 1970-01-01T00:00:00Z, POSIX scale
pub struct CivilTime { pub year: i32, pub month: u8, pub day: u8,
                       pub hour: u8, pub minute: u8, pub second: u8 }
pub struct Instant(u64);    // monotonic microseconds, origin unspecified
pub struct Duration(u64);   // microseconds
```

- `civil.rs`: `days_from_civil` and `civil_from_days` as integer
  arithmetic without floating point, the proleptic Gregorian leap-year
  rule, month lengths, and range validation of every field. The years run
  from 0 to 9999, which is what a `GeneralizedTime` can write down and
  therefore the widest range a certificate can name; a date outside them
  is an error and not a wrap.
- `unix.rs`: conversion in both directions, ordering, and `checked_add`
  and `checked_sub` of a `Duration`. Out-of-range results are errors,
  never wraps. A `UnixTime` resolves seconds, so the part of a `Duration`
  below one second does not move it, which the method documentation
  states rather than leaves to be discovered.
- `instant.rs`: monotonic time for timers. `Instant + Duration`,
  `saturating_duration_since`, and comparison. The crate reads no clock;
  the caller supplies the value. The `Add` implementation saturates,
  because a timer that saturates fires late while one that wraps fires
  immediately and forever; `checked_add` is there for a caller that wants
  to see the end instead.

The March-based arithmetic is what makes the calendar one expression
rather than a table: a year that begins in March ends with its leap day,
so the day of the year needs no month lengths. Every intermediate value
is bounded by the year range the module validates before it computes,
which is why its wrapping operators never wrap.

The generators of `CivilTime`, `UnixTime`, `Instant`, and `Duration` live
in the crate behind the feature `test-strategies`, where track D and
`fs-fat` will find them.

Leap seconds do not exist on the POSIX scale and time zones are not
modeled. Both are stated in the crate documentation as limits rather
than left implicit.

`UnixTime` is the type that document 11 attributes to `audhsos-der`. It
moved here, and `audhsos-der` produces a `CivilTime` from `UTCTime` and
`GeneralizedTime` instead of defining one, so that the certificate
validity window and the network timers speak one type (D-46). The parser
kept the syntax and gave up the field ranges: a `UTCTime` naming the
thirty-first of April or the twenty-ninth of February of a year that is
not leap is refused, which the old check against thirty-one let through.

Tests: catalog 6.6.39.

### 12.5.2 `audhsos-encoding`

Implemented.

- `base64.rs`: RFC 4648 alphabet, strict decoding — correct padding,
  no whitespace, and non-canonical trailing bits rejected. Strictness is
  the property that one sequence of bytes has one text: a decoder that
  skipped whitespace or ignored the unused bits of the last quantum would
  let a certificate be written two ways and compared once.
- `hex.rs`: lower-case encoding, case-insensitive decoding, odd length
  rejected. The asymmetry is deliberate — a hex dump is written by a
  program and read by a person, so the output is canonical while the
  input is not made to shout.
- `pem.rs`: RFC 7468 strict form — the label must match between the
  begin and end lines, lines are 64 characters except the last, a pad may
  stand only in the last of them, and trailing data after the end line is
  rejected. Explanatory text *before* the begin line is skipped, which the
  RFC permits and a certificate file with a preamble needs; text after it
  is not the same thing, because appending to a file is how a reader is
  made to see what the writer did not sign. Decoding yields the label and
  the DER bytes as borrowed slices, the first into the input and the
  second into a caller-supplied buffer.

A text with no end line reports the missing line rather than the length
of a body line: the body is located before it is read, so that the rule
for the last line is applied only to a block that has one.

Every function writes into a buffer the caller owns and returns the
number of bytes written; nothing allocates. The trust-anchor conversion
of D-42 uses this crate instead of an ad-hoc decoder in the xtask
(D-47), and the certificate builder of document 11 uses it to emit test
data in a form a human can read. Neither consumer exists yet, so neither
the xtask nor `audhsos-x509` depends on this crate today; when they do,
there is nothing ad hoc for them to replace, which is the point of
writing it first.

Tests: catalog 6.6.40. Fuzz target `pem`.

### 12.5.3 `audhsos-collections`

```rust
pub struct ArrayVec<T, const N: usize>;
pub struct RingBuffer<T, const N: usize>;
pub struct BitSet<const BITS: usize>;
pub struct IndexList;                        // intrusive list over caller-owned nodes
pub struct IndexMap<K: Ord, V, const N: usize>;
```

Fixed capacity, no allocation, no `unsafe`, and no panic: `push` on a
full container returns `Err(Full)`, and every accessor returns `Option`.
`IndexList` is a doubly linked list whose links are `u32` indices into a
slice the caller owns, which is exactly the shape the run queues of
phase 5 and the endpoint wait queues of phase 6 need, and which would
otherwise be written again in every crate that needs it (D-48).

Each container is tested against a reference model — `Vec`, `VecDeque`,
`BTreeMap` — with the model-test runner, which is what makes writing
them cheap.

Tests: catalog 6.6.41.

## 12.6 Track D: the network stack

### 12.6.1 Shape

The stack is sans-I/O in the sense of D-41: it consumes frames and
produces frames, it never blocks, it never allocates, and it knows
nothing about a device. Time enters as an `Instant`, randomness as the
`Rng` trait of `crypto-rng`, and buffers belong to the caller. Every
output is a function of state, input, and `now`, and the stack answers
`poll_at()` with the instant at which it next has work, so a server
never polls in a loop and a test never sleeps.

What is *not* in this track: `driver-virtio-net`, `server-net`, the
socket protocol in `user-proto`, and the entropy system call. They are
integration, they need the kernel, and section 8.14 keeps them
unscheduled.

### 12.6.2 Crate catalog

| Crate | Path | Layer | Depends on |
|-------|------|-------|------------|
| `net-wire` | `crates/net/wire` | n0 | - |
| `net-eth` | `crates/net/eth` | n1 | `net-wire`, `audhsos-time`, `audhsos-collections` |
| `net-ip` | `crates/net/ip` | n2 | `net-eth` and below |
| `net-udp` | `crates/net/udp` | n3 | `net-ip` and below, `crypto-rng` |
| `net-tcp` | `crates/net/tcp` | n3 | `net-ip` and below, `crypto-rng` |
| `net-dns` | `crates/net/dns` | n4 | `net-udp` and below, `crypto-rng` |
| `net-dhcp` | `crates/net/dhcp` | n4 | `net-udp` and below, `crypto-rng` |
| `net-http` | `crates/net/http` | n4 | `net-wire` |
| `net-stack` | `crates/net/stack` | n5 | all of the above |

`crypto-rng` is the only edge into track C, and it exists because
initial sequence numbers, ephemeral ports, and transaction ids must not
be guessable (D-51). `audhsos-tls` and the network crates never reference each other;
the transport glue that joins them is step T8 of document 11 and is
unscheduled.

### 12.6.3 `net-wire`

Address and header primitives shared by every layer above:
`MacAddr`, `Ipv4Addr`, `Ipv4Cidr`, `Port`, and the `EtherType` and
`Protocol` tables; a bounds-checked big-endian reader and writer that
returns `Result` instead of panicking; and the internet checksum of
RFC 1071 including the pseudo-header form that UDP and TCP need.

Every header type in the track is a borrowed view over a byte slice
(`Ipv4Packet<'a>`, `TcpSegment<'a>`), never a copy, in the form
`audhsos-x509` uses for certificates.

### 12.6.4 `net-eth`

Ethernet II frames: the 14-byte header, an MTU of 1500, no VLAN tags in
the first version. Frames shorter than the header, and frames whose
ether type is not registered, are dropped rather than rejected loudly.

ARP: request and reply encoding; a cache of fixed capacity with per
entry age, state (`Incomplete`, `Reachable`, `Stale`), and at most one
pending packet per destination; the retransmission schedule as a
function of `Instant`; gratuitous ARP accepted for refresh but never
allowed to replace a reachable entry with a different address, which is
the cheap half of ARP-spoofing resistance and costs one comparison.

### 12.6.5 `net-ip`

- IPv4 header parsing and writing; options are skipped, never
  interpreted; the header checksum is verified on receipt and computed
  on send; a TTL of zero is dropped.
- Reassembly: a fixed number of reassembly buffers, each with a deadline
  after which it is discarded; overlapping fragments discard the whole
  datagram; the total length is bounded before the first byte is copied.
- Fragmentation on send up to the interface MTU, with the don't-fragment
  bit honored.
- ICMPv4: echo request and reply; destination unreachable and time
  exceeded are parsed and delivered to the upper layer, so that a TCP
  connection to a closed port fails fast instead of timing out; generated
  errors are rate-limited by a token bucket.
- Routing: a table of fixed capacity with longest-prefix match, a
  default route, and on-link detection.

IPv6 is not in the first version. It is a boundary, not an oversight
(D-50).

### 12.6.6 `net-udp`

A fixed number of sockets, each bound to an explicit port or to an
ephemeral port drawn from `Rng` within a documented range; a receive
ring over caller-supplied memory; checksum verified when non-zero and
always written on send; broadcast permitted, because DHCP needs it.

### 12.6.7 `net-tcp`

The largest single piece of the track, and the reason it is XL.

- The complete state machine of RFC 9293: all eleven states, active and
  passive open. Passive open is included although the first client needs
  only active open, because two instances of a complete machine can be
  connected back to back over a simulated network, which is the test
  that makes the rest trustworthy.
- Sequence arithmetic modulo 2^32 with the `SEG.SEQ` window comparisons
  as their own tested module; initial sequence numbers per RFC 6528 from
  `Rng` and the clock.
- Send and receive windows over caller-supplied ring buffers; the
  receive window is advertised from the free space, and a window that
  would shrink is clamped rather than retracted.
- Retransmission per RFC 6298: SRTT and RTTVAR, a minimum RTO of one
  second, a maximum of sixty, exponential backoff, and Karn's rule for
  retransmitted segments. Fast retransmit after three duplicate
  acknowledgments.
- Congestion control per RFC 5681: slow start, congestion avoidance,
  fast recovery.
- Delayed acknowledgments bounded at 500 milliseconds and at every
  second full-sized segment; Nagle's algorithm with an off switch; a
  persist timer for zero-window probing. No keepalive in the first
  version.
- Options: maximum segment size only. No window scaling, no selective
  acknowledgment, no timestamps. The cost is throughput over a path with
  a large bandwidth-delay product, which QEMU's virtual link does not
  have; the boundary is stated so that the omission is not mistaken for
  a bug (D-50).
- `TIME-WAIT` for twice the maximum segment lifetime, with the lifetime
  a documented constant of thirty seconds.
- Reset handling with the RFC 5961 checks: an in-window test for
  incoming resets and SYNs, and a challenge acknowledgment rather than a
  blind teardown.

### 12.6.8 `net-dns`

Message encoding and decoding with name compression on read — bounded
jumps and loop detection, because a compression pointer loop is the
classic denial of service of this format — and without compression on
write. `A` records and `CNAME` chains up to depth eight; other types are
parsed as opaque and ignored. The resolver is a state machine over UDP
with retry, server rotation, and a deadline; the transaction id and the
source port come from `Rng`, and a response is accepted only when id,
question section, source address, and port all match.

### 12.6.9 `net-dhcp`

The four-message exchange, the options the stack needs (subnet mask,
router, DNS servers, lease time, server identifier, message type), and
the lease state machine with T1 renewal, T2 rebinding, and expiry. The
transaction id comes from `Rng`; retransmission backs off exponentially
with jitter, as RFC 2131 requires.

### 12.6.10 `net-http`

An HTTP/1.1 client: request line and headers encoded into a
caller-supplied buffer; the response parsed strictly — a bounded status
line, a bounded number of headers of bounded length, no obsolete line
folding, and `Content-Length` together with `Transfer-Encoding` rejected
outright, which is the rule that closes request smuggling. Chunked
transfer decoding is supported. Redirects are reported to the caller,
never followed. No content encodings in the first version.

### 12.6.11 `net-stack`

The facade: an `Interface` with its MAC address, its addresses, its
routes, and its MTU; `poll(now, rx, tx)` which demultiplexes an incoming
frame down the layers and drains the outgoing work; socket handles as
generation-checked indices, in the form of the kernel's handles; and
`poll_at(now)`. The whole stack has one entry point, so a server process
is a loop around it and nothing else.

### 12.6.12 Testing

- Vector tests for every header format, taken from the RFCs. Frames used
  in tests are constructed by project code; no capture from a foreign
  device enters the repository (D-40 applies unchanged).
- Property tests: no byte stream makes a parser panic, and every
  accepted packet re-encodes to its input bytes.
- Model tests for TCP: two instances connected through a network double
  that delays, duplicates, reorders, and drops, with the invariant that
  every byte handed to one side arrives once, in order, at the other, and
  that both sides reach `CLOSED`. The same runner drives the ARP cache
  and the reassembly buffers against reference models.
- Fuzz targets `ipv4`, `tcp_segment`, `dns_message`, and
  `http_response`.
- No test sleeps. Time is an argument, so a sixty-second retransmission
  backoff is exercised in microseconds of wall clock.

Tests: catalog 6.6.42 to 6.6.50.

### 12.6.13 Order of work

| Step | Content | Size |
|------|---------|------|
| D1 | `net-wire`: addresses, cursor, checksums | S |
| D2 | `net-eth`: frames and the ARP cache | M |
| D3 | `net-ip`: header, reassembly, fragmentation, ICMP, routes | M |
| D4 | `net-udp` | S |
| D5 | `net-tcp`: sequence arithmetic, state machine, timers, congestion control | XL |
| D6 | `net-dns` and `net-dhcp` | M |
| D7 | `net-http` | S |
| D8 | `net-stack`: the facade | M |
| D9 | integration, jointly with T8 of document 11: `driver-virtio-net`, `server-net`, the socket protocol, the entropy system call, the TLS transport | not scheduled |

D5 is the one step that must not be started beside an XL phase.

## 12.7 Track F: device logic without devices

Two components that a driver needs and that contain no device access.

### 12.7.1 `virtio-queue`

The split virtqueue of virtio 1.x as data-structure logic over a
`QueueMemory` trait with a test double: the descriptor table, the
available and used rings, descriptor chains, index arithmetic modulo the
queue size, and the notification suppression flag. Separately, the
device initialization state machine — reset, acknowledge, driver,
feature negotiation, features-ok, driver-ok — with the failure path into
`DEVICE_NEEDS_RESET`.

No memory-mapped register touches this crate; the adapter that maps a
device does, and it lives in a driver process later. Packed rings,
indirect descriptors, and `EVENT_IDX` are not in the first version
(D-52).

Tests: catalog 6.6.51.

### 12.7.2 `fs-fat`

The xtask already contains a FAT32 writer (D-09, catalog 6.6.15). Its
structural logic moves into `fs-fat` over a `BlockDevice` trait with a
RAM-disk double: boot parameter block validation, FAT chain traversal,
cluster allocation, directory entries in 8.3 form, and file read and
write. The xtask image writer becomes a user of the crate, and the file
system server that section 8.14 leaves unscheduled becomes a second one.
This removes the only duplication the tooling has planned for itself.

FAT32 only, no FAT12 or FAT16, no long file names (D-09 unchanged),
timestamps through `audhsos-time` (D-53).

Tests: catalog 6.6.52.

## 12.8 Track G: tooling

### 12.8.1 `fuzz-support`

Implemented. The `LLVMFuzzerTestOneInput` entry glue, which is the
allowlisted `unsafe` of this crate, a `fuzz_target!` macro, corpus
handling, and the regression replay that `cargo xtask fuzz --regression`
runs as the last step of `check`.

The one `unsafe` is `input(data, len)`, which turns the fuzzer's pointer
and length into a slice and answers the empty slice for a length of zero
or a null pointer, the two cases `from_raw_parts` does not allow. Miri
covers it.

A target is one source built two ways. With the coverage instrumentation
and `--cfg fuzzing` it is a libFuzzer binary whose `main` comes from the
runtime; without them it is an ordinary program that replays the files
named on its command line, which is how a corpus is checked on a machine
that has no fuzzer runtime. `fuzz/` is a workspace of its own, excluded
from the root one, because those flags are not the flags of the checks.

The targets that exist are the parsers that exist: `elf`,
`boot_image_header`, and `boot_info`. `madt` follows in Phase 4, `tar` and
`message` in Phase 5 and Phase 7. Every crash becomes a regression test in
the parser's crate and its input a file under `fuzz/corpus/<target>/`.

### 12.8.2 `audhsos-symbols`

Implemented. A reader for the symbol table and the DWARF line program of
an ELF file, built on `audhsos-elf`: address to function, file, and line.

`audhsos-elf` grew the section header table for it, which the loader does
not read and a symbolizer cannot do without; `sections()` validates the
magic, the class, and the byte order and nothing about segments, so a file
without a loadable segment still yields its sections.

The line program runs its state machine once per lookup and keeps only the
row it needs, so the crate allocates nothing and borrows everything from
the bytes it was handed. A row counts for an address only inside the
sequence that ends above it. Version 4 and version 5 are read, the latter
with the forms a file table uses: `string`, `strp`, `line_strp`, `udata`,
the four fixed widths, `data16`, and `block`; an unknown form is an error
rather than a guess. No inline frames, no call-frame information and
therefore no stack unwinding (D-54).

The directory and the file come back separately, because joining them
would mean allocating; the xtask joins them.

`demangle` writes a name back readable, in the `v0` scheme of RFC 2603 and
the legacy `_ZN` scheme, straight into a formatter and therefore without
allocating. Generic arguments are dropped, because a report wants the path
and not the instantiation, and a name the parser does not understand is
written unchanged. Every one of the 1002 symbols of the kernel image reads
back.

The xtask uses it two ways. `cargo xtask symbolize <elf> <address>...`
answers by hand, and a QEMU run that fails resolves every address of the
kernel half in its serial output against the image it ran, after it prints
that output. A file that cannot be read or carries no symbols produces
nothing: the report is a comment on a run that already failed and must not
fail it a second time.

Tests: catalog 6.6.53.

## 12.9 Phase work that may be pulled forward

These components belong to phases and stay there: their catalog entries,
their acceptance criteria, and their position in the roadmap do not
change. What changes is that their logic may be written at any earlier
time, because it passes the admission test. What remains in the phase is
the integration.

| Component | Phase | Catalog | Why it can be written now |
|-----------|-------|---------|---------------------------|
| `gfx` | 9 | 6.6.26 | draws into a byte buffer; a test owns the buffer, a framebuffer is not involved |
| `driver-i8042` | 10 | 6.6.25 | port access trait with a double, exactly as `driver-uart16550` today |
| QMP client and PPM reader in the xtask | 9 | 6.6.28 | protocol logic over a stream, tested against recorded sessions |
| allocator logic in `user-rt` | 7 | 6.6.12 | offsets in a byte region, testable against a reference model |
| encodings in `user-proto` | 7 | none yet | each message is a type with `encode` and `decode` and no system call; the catalog covers them only through the end-to-end items 6.6.22, so pulling them forward means writing a catalog item for them first |

## 12.10 Capacity

- At most one side track besides track C is active at a time. Three
  parallel tracks dilute attention, which is the failure mode section
  8.15 already names for track C (D-45).
- A track is worked on between phases, never instead of one.
- Recommended order: track E first, because it is small and blocks track
  C at T5 and T6; then track C to T7; then track D from D1; track F when
  a driver becomes foreseeable; `audhsos-symbols` from track G before
  phase 3, because that is where kernel panics start.
- The pulled-forward work of 12.9 fills short gaps, because it needs no
  new design.

## 12.11 Deliberately not started

Each item below would pass the admission test and has no consumer in the
current plan. They are listed so that they do not return as ideas.

DEFLATE and gzip; a VT100 terminal emulator and a line editor; a JSON
parser beyond the QMP subset the xtask needs; IPv6; the TLS server role;
the bignum crate that RSA verification needs (document 11); USB;
compression or encryption of the boot image.

## 12.12 Risks

| Risk | Effect | Mitigation |
|------|--------|------------|
| Parallel work displaces the phases | the release slips while the workspace grows | at most one side track beside track C; a track is never worked on instead of a phase (D-45) |
| TCP is underestimated | D5 stalls the track | the state machine, the timers, and the congestion control are separate modules with separate catalog items; the back-to-back model test exists before the first timer is tuned |
| A stack written without a device meets a real device badly | rework when the driver arrives | every layer is a borrowed view over bytes with no assumption about who produced them; the virtqueue logic of track F is written before the driver, not with it |
| Foundations arrive after their consumers | the containers and the time arithmetic get written twice | track E is scheduled first and is small |
| The FAT32 move breaks the image writer | phase 2 tooling regresses | the move is a refactoring with the existing catalog 6.6.15 tests kept green, followed by the new tests of 6.6.52 |
| Options omitted from TCP are read as defects later | avoidable confusion | the omissions are decisions (D-50) and appear in the crate documentation |
