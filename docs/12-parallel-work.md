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
| E | shared foundations: time, encodings, collections (12.5) | M | implemented; needed by C at T5 and T6, by D throughout, by phases 5 and 6 |
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

Implemented.

```rust
pub struct ArrayVec<T, const N: usize>;
pub struct RingBuffer<T, const N: usize>;
pub struct BitSet<const WORDS: usize>;       // WORDS * 64 bits
pub struct IndexList;                        // intrusive list over caller-owned nodes
pub struct Link;                             // the two links and the owner of one node
pub struct IndexMap<K: Ord, V, const N: usize>;
```

Fixed capacity, no allocation, no `unsafe`, and no panic: `push` on a
full container returns `Err(Full)`, and every accessor returns `Option`.
`IndexList` is a doubly linked list whose links are `u32` indices into a
slice the caller owns, which is exactly the shape the run queues of
phase 5 and the endpoint wait queues of phase 6 need, and which would
otherwise be written again in every crate that needs it (D-48).

Two shapes differ from the sketch above, and both are decisions rather
than accidents.

`BitSet` is parameterised by its number of 64-bit words and not by its
number of bits, because `[u64; BITS.div_ceil(64)]` is an expression over
a const parameter and stable Rust cannot size an array with one. The
alternative was an incomplete language feature in the crate every other
crate rests on, which is the worse trade. `BitSet::BITS` reports the size
and every operation is checked against it, so a caller reads the size
from the type rather than computing it.

`IndexList` carries an identifier and a [`Link`] records which list its
node is in. Several lists over one slice is the shape of one run queue
per priority, and without the identifier a node handed to the wrong list
would be quietly stolen from the right one; with it, that is
`NotLinked`. `NONE` is the index that names no node and therefore cannot
name a list, which is why `IndexList::new` is fallible.

The owning containers store `Option<T>`. That costs one discriminant per
slot and buys the right to hold a `T` with no default without a line of
`unsafe`; the price is that there is no `as_slice`, because handing out a
`&[T]` over storage the type system believes may be empty is exactly what
`unsafe` exists for. `iter` is the way through.

Each container is tested against a reference model — `Vec`, `VecDeque`,
`BTreeMap`, and a `BTreeSet` of indices for the bits — with the
model-test runner, which is what makes writing them cheap.

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

The stack carries both families (D-69). An address is an `IpAddr`
everywhere above `net-wire`, a route is an `IpCidr`, and a neighbor is one
cache entry whichever protocol resolved it; the two internet layers are
separate crates because their headers, their errors, and their address
configuration have almost nothing in common, and everything above them is
written once.

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
| `net-ipv6` | `crates/net/ipv6` | n2 | `net-ip` and below |
| `net-udp` | `crates/net/udp` | n3 | `net-wire`, `crypto-rng` |
| `net-tcp` | `crates/net/tcp` | n3 | `net-ip` and below, `crypto-rng` |
| `net-dns` | `crates/net/dns` | n4 | `net-udp` and below, `crypto-rng` |
| `net-dhcp` | `crates/net/dhcp` | n4 | `net-udp` and below, `crypto-rng` |
| `net-http` | `crates/net/http` | n4 | `net-wire` |
| `net-stack` | `crates/net/stack` | n5 | all of the above |

`net-ip` holds what the two families share — the routing table over
`IpCidr`, the reassembly machinery, and the interface a packet leaves
through — and the IPv4 half; `net-ipv6` holds the IPv6 half. Neither of
the two senders is a transport's concern: a transport writes its segment
into a buffer and names the two addresses it wrote it for, and the facade
of 12.6.12 picks the sender that carries it out. A transport therefore
reaches no further down than `net-wire`, which is what makes a socket one
piece of code and what keeps `net-udp` from depending on `net-ip` at all.

`crypto-rng` is the only edge into track C, and it exists because
initial sequence numbers, ephemeral ports, and transaction ids must not
be guessable (D-51). `audhsos-tls` and the network crates never reference each other;
the transport glue that joins them is step T8 of document 11 and is
unscheduled.

### 12.6.3 `net-wire`

Implemented.

Address and header primitives shared by every layer above:
`MacAddr`, `Ipv4Addr`, `Ipv4Cidr`, `Port`, and the `EtherType` and
`Protocol` tables; a bounds-checked big-endian reader and writer that
returns `Result` instead of panicking; and the internet checksum of
RFC 1071 including the pseudo-header form that UDP and TCP need.

Every header type in the track is a borrowed view over a byte slice
(`Ipv4Packet<'a>`, `TcpSegment<'a>`), never a copy, in the form
`audhsos-x509` uses for certificates. `Reader` hands out those borrows,
so nothing above it copies a frame to read one.

- `addr.rs`: the four address types. The text form is canonical in both
  directions — what `Display` writes is what `parse` reads, and a leading
  zero, a fifth group, a missing group, or a prefix length above 32 is an
  error (D-68). `Ipv4Cidr` keeps the address it was given rather than the
  network, because `192.168.1.7/24` is an interface address and
  `network()` is the separate question; its mask arithmetic goes through
  `checked_shr`, so a prefix of 32 does not shift by the whole width.
- `cursor.rs`: `Reader` and `Writer`. Every operation either takes
  exactly what it asked for or takes nothing and leaves the position
  where it was, so a parser that runs out of bytes reports and stops
  instead of unwinding. `Writer::patch_u16` is the one backwards write,
  and it exists because a checksum covers the header it sits in: the
  field is written as zero, the header is finished, the sum is taken over
  `written()`, and the answer goes back.
- `protocol.rs`: `EtherType` and `Protocol` as wrappers over the number
  on the wire rather than enumerations, so a value this system has no
  layer for is a value and not a parse error — which is what lets
  `net-eth` drop an unregistered frame quietly, as 12.6.4 requires,
  instead of failing to read it.
- `checksum.rs`: the accumulator carries the end-around carry at every
  word instead of deferring it as RFC 1071 section 2 recommends, so it
  provably stays inside sixteen bits and every addition is checked; and
  it holds the odd byte a call ended on, so a pseudo-header, a header,
  and a payload may arrive in three calls (D-68).

Both families are here from the start (D-69). `Ipv6Addr` reads and writes
the canonical text of RFC 5952, section 4 and refuses every other spelling
RFC 4291 permits, including the dotted form of an IPv4-mapped address,
which this system does not carry as a value at all; `Ipv6Addr::solicited_node`
derives the multicast group of RFC 4291, section 2.7.1, which is where
Neighbor Discovery asks. `IpAddr`, `IpCidr`, and `IpVersion` are the types
every layer above carries. The checksum has the pseudo-header of RFC 8200,
section 8.1 beside the IPv4 one — thirty-two bits of length, and the
upper-layer protocol rather than the packet's next-header field — and
`transport` dispatches on the family and answers `MixedFamilies` for a
pair that is not one. `Protocol` gained `ICMPV6` and the four extension
header numbers, and `has_pseudo_header` knows that `ICMPv6` sums over one
where `ICMPv4` does not.

The crate has no `test-strategies` feature. Generators for addresses have
no consumer until `net-eth` needs them, and the crate's own property
tests generate their bytes with `test-support` directly.

RFC 1071 is kept under `docs/rfc/` with the other reference documents, on
the arrangement of D-59; its section 3 is the worked example the vectors
are transcribed from.

### 12.6.4 `net-eth`

Implemented.

Ethernet II frames: the 14-byte header, an MTU of 1500, no VLAN tags in
the first version. Frames shorter than the header, and frames whose
ether type is not registered, are dropped rather than rejected loudly.

The neighbor cache: one cache of fixed capacity for both families, keyed
by `IpAddr`, with per entry age, at most one pending packet per
destination, and the states of RFC 4861 — `Incomplete`, `Reachable`,
`Stale`, `Delay`, `Probe` — of which ARP uses the three it needs. The
retransmission schedule is a function of `Instant`.

ARP fills the IPv4 half from here: request and reply encoding, with
gratuitous ARP accepted for refresh but never allowed to replace a
reachable entry with a different address, which is the cheap half of
ARP-spoofing resistance and costs one comparison. The IPv6 half is filled
by Neighbor Discovery, which is `ICMPv6` and therefore lives in
`net-ipv6`; it writes into this cache rather than keeping a second one
(D-69). The cache is thus driven from above in both cases, and `net-eth`
owns the storage and the timers and neither of the two protocols.

Two things came with D4. `on_conflict` is the second half of the rule
`on_observed` implements: RFC 4861, section 7.2.5 I has a neighbor
advertisement without the override bit leave a disagreeing hardware
address alone, and take a `Reachable` entry to `Stale` all the same. The
cache cannot make that call for itself — it does not know the bit exists
— so the decision is Neighbor Discovery's and the state change is this
crate's. What it buys is that a contradicted mapping is checked before
the next packet rather than trusted for the rest of the reachable time.

`multicast_hardware` is the other, and the one addition this crate took
for it: the mapping of RFC 2464, section 7 from an IPv6 multicast group
onto the Ethernet address it is reached at. It belongs here and not in
`net-ipv6`, because it is a fact about an Ethernet and not about IPv6 —
the same layer that knows a frame is fourteen bytes and carries 1500.
Being arithmetic and not a table, it needs no membership list: a
solicitation goes to the solicited-node group of a station this host has
never heard of, which is exactly the case ARP has to broadcast for.

Three things came out of writing it.

Reception is a filter and not a parse: `receive` answers `Option` and
drops a frame that is short, oversized, addressed elsewhere, or of a type
no layer here reads. `Frame::parse` keeps the `Result` for the two
structural failures, so a caller that wants to know why can ask; the
receive path does not, because a link carries other stations' traffic and
a stack that reported each piece of it would report nothing worth
reading.

The destination filter takes any multicast group and not the ones this
station joined. The groups an IPv6 host belongs to follow from the
addresses it holds, which this layer does not know, so membership is
checked one layer up where it is. What that costs is parsing a multicast
frame that is then dropped; what it buys is that this layer keeps no list
in step with another.

The cache carries the reachable time as it is given. RFC 4861,
section 6.3.2 draws it afresh from a factor between one half and one and
a half so that hosts which learned a neighbor together do not re-probe
together, and this crate takes no randomness — it would need `crypto-rng`
in the layer furthest from needing one. The cost is synchronised probes
among hosts that booted together, and it is written in the crate rather
than left to be discovered.

The protection against a stolen mapping is one comparison and no more:
`Reachable` is the only state an unsolicited claim cannot change, since
that station answered a solicitation of this host's and an unsolicited
claim is no evidence at all. An entry in any other state goes to whoever
claims it last. Knowing which station is entitled to an address is not
something a link layer can know, and the crate says so rather than
implying more.

### 12.6.5 `net-ip`

Implemented.

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
- Routing: a table of fixed capacity over `IpCidr` with longest-prefix
  match, a default route per family, and on-link detection. The table
  holds both families and a route of one never matches a destination of
  the other.
- The interface the transports send through, which takes an `IpAddr` pair
  and a protocol and knows nothing about which family will carry it.

The reassembly machinery is here and not in `net-ipv6`, because the two
differ in where the fragment fields sit and in nothing else that a
reassembly buffer cares about. D4 made that concrete and changed three
things here, all of them narrowing rather than widening. `Reassembler`
works in a `Piece`, which is what either family's header describes once
the family-specific part has been read: the two addresses as `IpAddr`,
the identification widened to the thirty-two bits IPv6 uses, the offset,
and the more bit. `Fragments::with_header` takes the header length as an
argument, because IPv4 puts twenty bytes in front of a piece and IPv6
forty-eight, and everything behind that number is one piece of
arithmetic. And the reassembled datagram no longer carries a copy of the
first fragment's header: the caller holds the piece that completed it,
the fields reassembly is keyed on are equal in every piece by definition,
and the ones that are not — the time to live, the traffic class, IPv4's
options — are read by no layer above. What that removed was a twenty-byte
copy per buffer and the flag that said whether it had happened.

Four things came out of writing it.

An ICMP error quotes a header and eight bytes, so the total length that
header declares is longer than what is there and it cannot be read as a
datagram. `Quoted::parse` reads it where `Datagram::parse` correctly
refuses to: the version, the header length, and the checksum are checked
as they are in a whole datagram, and only the total length is not. That
is what lets a transport find the connection an error belongs to.

An overlapping fragment discards the datagram. RFC 791 leaves the case
open, and every answer but discarding lets a filter be walked past — a
first fragment shows one transport header and a second overwrites it. A
duplicate that agrees byte for byte is dropped instead, and costs
nothing.

`Fragments` carries a `next` and is deliberately not an `Iterator`. It is
`Copy`, and an iterator that is copied silently starts again, which is
the one shape of this type that would surprise a reader.

The send path copies each piece once into its frame. Writing the link
header into the front of the same buffer and letting the fragmenter write
behind it would save the copy and would mean this crate knowing the
layout of an Ethernet frame instead of asking `net-eth` for it; one
memcpy of at most an MTU is the cheaper of the two on a path where the
driver copies again anyway.

The send path carries IPv4 only for now. `Sender::send` takes an `IpAddr`
pair and refuses one of the second family, because the header it writes
is the IPv4 one; `net-ipv6` writes its own and then asks this same
routing table and the same neighbor cache. The signature is the one both
families will use, so nothing above it changes when D4 lands.

### 12.6.6 `net-ipv6`

Implemented.

- Header parsing and writing, and the extension header chain: hop-by-hop
  options, routing, fragment, and destination options are stepped over to
  reach the upper-layer header. The walk is bounded in the number of
  headers and in the bytes it may consume, and a chain that does not end
  is a drop — an unbounded chain is this format's version of the
  compression pointer loop of 12.6.9.
- No header checksum, because IPv6 has none, and therefore no
  verification on receipt: a corrupted header is caught by the
  upper-layer checksum, which is why RFC 8200 makes that one mandatory
  where IPv4 left it optional for UDP.
- Fragmentation on send through the fragment header, and reassembly
  through the machinery of `net-ip`. A router never fragments an IPv6
  packet, so the source does it or it does not happen.
- `ICMPv6` per RFC 4443: echo request and reply; destination unreachable,
  packet too big, and time exceeded parsed and delivered upward. Its
  checksum covers the pseudo-header, which `ICMPv4`'s does not.
- Neighbor Discovery per RFC 4861: solicitation and advertisement,
  addressed to the solicited-node multicast group rather than broadcast,
  writing into the neighbor cache of `net-eth`; duplicate address
  detection before an address is used.
- Router advertisements and SLAAC: a prefix and a router learned from an
  advertisement, an address formed from the prefix, the lifetimes that go
  with them, and the recursive DNS server option of RFC 8106. There is no
  DHCPv6 (D-69).
- Path MTU discovery, driven by the packet-too-big message. It is not
  optional here: no router will fragment for this host.

Seven things came out of writing it, and D-72 records the boundaries
three of them draw.

A router advertisement cannot expire an address this host holds. That is
the rule of RFC 4862, section 5.5.3 (e), and the document says plainly
what it is for: without it one forged advertisement carrying a valid
lifetime of a second, or of none, would take a host that configured
itself from the network off it again with a single packet. An
advertisement may lengthen a lifetime freely and shorten it to anything
above two hours; below that it may only shorten an address that had less
than two hours left anyway, and legitimate advertisements, being
periodic, cancel a short lifetime long before it takes effect. The floor
here applies to the whole entry and not to the address alone, so a router
that withdraws such a prefix is honoured after at most two hours for the
on-link route as well. A prefix this host formed no address under has
nothing to protect and is withdrawn at once, as RFC 4861, section 6.3.4
has it.

Nothing is removed behind the caller's back. A withdrawn prefix or DNS
server has its lifetime set to the present instant and leaves through
`poll` like every other lifetime that runs out, because the caller
installed a route for that prefix and has to hear that it must go. The
first version removed it silently and left the route standing.

The chain walk is bounded twice and neither bound is the packet. Eight
headers and 512 bytes end it, and running out of packet ends it too;
every extension header is at least eight bytes, so the walk provably
shrinks its input and cannot spin. The two bounds are what stop it before
it has read a kilobyte of headers to reach a transport header that is not
there, which is the cheap denial of service this format offers. What they
cost is a chain longer than any correct sender writes: RFC 8200
recommends at most one of each kind, and there are six kinds.

Path MTU discovery is in the send path and not beside it. `Sender` holds
the estimate table and asks it for every packet, so a packet larger than
what the path is known to carry is one this crate cannot write. The
alternative — a caller that lowers `Interface.mtu` when it remembers to —
is one forgotten call away from frames that vanish, and there is no
router downstream to turn them into an error.

The estimate has its floor where the message arrives. RFC 8201 says the
estimate is never taken below 1280, and this is implemented by discarding
a packet-too-big message that reports less, not by clamping the answer:
so a link whose own MTU is below 1280 is reported as it stands and the
send path refuses it. Such a link cannot carry IPv6 at all, and answering
1280 for it would turn a clear error into frames a driver silently
cannot send.

`ICMPv6` is parsed twice over. `Message::parse` reads the structure and
`Message::parse_checked` verifies the checksum first, and both are
public. The split is not a convenience: the checksum needs the two
addresses, so a caller holding a message without the packet it arrived
in — the fuzz target, a test, a quoted message inside an error — cannot
verify one, and giving it only the checked entry point would mean either
inventing addresses or not testing the parser at all.

Neighbor Discovery checks the hop limit before anything else. RFC 4861,
section 7.1 requires 255, and it is the whole of what makes the protocol
link-local: a message carries no other proof of where it came from, and
every router on the way decrements the field. A host that skips the check
accepts neighbor advertisements, and therefore hardware addresses for its
neighbors, from the whole internet. The check is in `receive`, which is
the only door into this module from a packet.

The link-layer address option is read at exactly six bytes and no other
length. RFC 4861 gives it one unit for a link whose addresses are six
bytes, and reading the first six of a longer body would be reading an
option for a link this system has no frames for. A body of the wrong
length reads as an unrecognized option, which the document has a receiver
ignore, rather than as a reason to drop the message: the length field was
right, so the option walk is not lost.
### 12.6.7 `net-udp`

Implemented. A fixed number of sockets, each bound to an explicit port or
to an ephemeral port drawn from `Rng` within a documented range; a receive
ring over caller-supplied memory; broadcast permitted, because DHCP needs
it. A socket carries an `IpAddr` and no family of its own.

The checksum differs by family and the difference is not cosmetic: over
IPv4 it is verified when non-zero and may be omitted on send, over IPv6
it is mandatory in both directions, because there is no header checksum
underneath it to catch a corrupted address (RFC 8200, section 8.1). A
datagram whose sum comes out zero is sent as all ones in both.

Four things the specification left open, as D-74 decided them:

- **The length field is what is believed.** Bytes behind it are padding
  that a link layer left standing — an Ethernet frame is padded to sixty
  bytes — and are cut away; a length that reaches past the bytes that
  arrived is an error. The checksum covers exactly what the length field
  claims, so a truncated view and its sum agree.
- **The ring holds self-describing records, not fixed slots.** Each record
  carries the two addresses the datagram arrived between, the port it came
  from, and its payload; a record that no longer fits behind the last
  begins at the front of the buffer rather than being cut in two, so a
  payload comes back as one slice. One buffer, one limit, and nothing
  wasted on a slot that is larger than the datagram in it. A full ring
  drops the newest datagram and counts the drop, and a datagram that would
  not fit an empty ring is refused at entry. The destination address is in
  the record because a socket bound to no address of its own serves every
  address of the host, and a reply that leaves from the wrong one is one
  the peer discards.
- **A port is held once, whichever address holds it.** A socket bound to
  one of this host's addresses takes only what named that address; a
  socket bound to none takes everything that reaches the port, broadcast
  and multicast included, which is the form a DHCP client needs. Two
  sockets on one port with different local addresses would turn a lookup
  into a precedence rule and buys nothing this system wants.
- **An ephemeral port that is taken is answered by drawing again**, as
  RFC 6056, section 3.3.1 asks, and not by walking to the next port: a
  port beside a taken one is a port an observer who saw the first can
  guess. The dynamic range of RFC 6335 is exactly `2^14` ports wide, so
  the low fourteen bits of two random bytes name one without the bias a
  remainder would introduce. The number of draws is bounded at eight.

The crate sends nothing and therefore depends on no internet layer. It
writes a datagram for a pair of addresses and reports a datagram for a
port nobody holds as such; which sender carries the one out, and whether
RFC 1122, section 3.2.2 allows an ICMP error for the other, are decisions
of the layers that have the header fields those rules are about.

### 12.6.8 `net-tcp`

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

### 12.6.9 `net-dns`

Message encoding and decoding with name compression on read — bounded
jumps and loop detection, because a compression pointer loop is the
classic denial of service of this format — and without compression on
write. `A` and `AAAA` records and `CNAME` chains up to depth eight; other
types are parsed as opaque and ignored. A name is asked for in both types
and the answers of both are returned, because which family a host reaches
a name over is the caller's question and not the resolver's. The resolver
is a state machine over UDP with retry, server rotation, and a deadline;
the transaction id and the source port come from `Rng`, and a response is
accepted only when id, question section, source address, and port all
match.

### 12.6.10 `net-dhcp`

IPv4 only. The four-message exchange, the options the stack needs (subnet
mask, router, DNS servers, lease time, server identifier, message type),
and the lease state machine with T1 renewal, T2 rebinding, and expiry. The
transaction id comes from `Rng`; retransmission backs off exponentially
with jitter, as RFC 2131 requires.

IPv6 configures itself from a router advertisement instead, which is
`ICMPv6` and therefore in `net-ipv6`. There is no DHCPv6 (D-69).

### 12.6.11 `net-http`

An HTTP/1.1 client: request line and headers encoded into a
caller-supplied buffer; the response parsed strictly — a bounded status
line, a bounded number of headers of bounded length, no obsolete line
folding, and `Content-Length` together with `Transfer-Encoding` rejected
outright, which is the rule that closes request smuggling. Chunked
transfer decoding is supported. Redirects are reported to the caller,
never followed. No content encodings in the first version.

### 12.6.12 `net-stack`

The facade: an `Interface` with its MAC address, its addresses of both
families, its routes, and its MTU; `poll(now, rx, tx)` which
demultiplexes an incoming frame down the layers and drains the outgoing
work; socket handles as generation-checked indices, in the form of the
kernel's handles; and `poll_at(now)`. The whole stack has one entry
point, so a server process is a loop around it and nothing else.

Source and destination address selection follows RFC 6724, which is what
decides which of a host's addresses a packet leaves with and which of a
name's addresses it goes to. Connecting to a name that resolves to both
families tries them in that order and does not race them: Happy Eyeballs
(RFC 8305) is a policy above the stack and not in it, and its absence
costs a timeout on a broken path rather than a wrong answer.

### 12.6.13 Testing

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
- The address types of both families round-trip through their canonical
  text, and no text this parser accepts has a second spelling (property).
- Fuzz targets `ipv4`, `ipv6`, `tcp_segment`, `dns_message`, and
  `http_response`. `ipv4` exists and drives four parsers, because a byte
  stream reaches each by a different door: the datagram, the quoted header
  an error carries, the `ICMPv4` message behind the payload, and the
  reassembler, which is the one with state. `ipv6` exists and drives four
  as well — the packet with its chain walk, the `ICMPv6` message, the
  Neighbor Discovery message with its option walk, and the reassembler —
  and it exists for the extension header chain, which is the one part of
  that format a byte stream can drive in circles. Its seeds are named in
  words, one per shape a test cites: the four headers in order, a chain
  without an end, a header that reaches past the packet, the two halves
  of a fragmented datagram, and one message of each `ICMPv6` and Neighbor
  Discovery kind.
- No test sleeps. Time is an argument, so a sixty-second retransmission
  backoff is exercised in microseconds of wall clock.

Tests: catalog 6.6.42 to 6.6.50 and 6.6.54.

### 12.6.14 Order of work

| Step | Content | Size |
|------|---------|------|
| D1 | `net-wire`: addresses, cursor, checksums — implemented | S |
| D2 | `net-eth`: frames, ARP, and the neighbor cache — implemented | M |
| D3 | `net-ip`: IPv4 header, reassembly, fragmentation, `ICMPv4`, the routing table over both families, the send path — implemented | M |
| D4 | `net-ipv6`: header and extension chain, `ICMPv6`, Neighbor Discovery, router advertisements and SLAAC, path MTU discovery — implemented | L |
| D5 | `net-udp` — implemented | S |
| D6 | `net-tcp`: sequence arithmetic, state machine, timers, congestion control | XL |
| D7 | `net-dns` and `net-dhcp` | M |
| D8 | `net-http` | S |
| D9 | `net-stack`: the facade, with the address selection of RFC 6724 | M |
| D10 | integration, jointly with T8 of document 11: `driver-virtio-net`, `server-net`, the socket protocol, the entropy system call, the TLS transport | not scheduled |

D6 is the one step that must not be started beside an XL phase.

D4 is what D-69 added, and D2 and D3 grew with it: the neighbor cache is
one cache for two protocols, and the routing table is one table for two
families. It is written before the transports rather than after them,
because a socket that has learned one family is a socket that has to be
widened.

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

Implemented. The fuzzing engine (D-63), a `fuzz_target!` macro, corpus
handling, and the regression replay that `cargo xtask fuzz --regression`
runs as the last step of `check`.

The `unsafe` of this crate is the boundary to the coverage
instrumentation and nothing else: `sancov` holds the callbacks the
compiler emits calls to, and `counters` turns the ranges the linker placed
into slices. The mutator, the corpus, the loop, the macro, and every fuzz
target above them are safe code. Miri covers what it can of it and skips
the tests that stand in for ranges the linker placed, which it has no way
to produce.

A target is one source built two ways. With `--cfg fuzzing` its `main` is
the engine's loop; without it, an ordinary program that replays the files
named on its command line, which is how a corpus is checked. `fuzz/` is a
workspace of its own, excluded from the root one, because the
instrumentation it is built with is not what the checks use.

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
  phase 3, because that is where kernel panics start. Tracks E and G are
  done, track C stands at T7, and track D has D1 to D5 behind it, so the
  next step of the side work is D6.
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
| TCP is underestimated | D6 stalls the track | the state machine, the timers, and the congestion control are separate modules with separate catalog items; the back-to-back model test exists before the first timer is tuned |
| A stack written without a device meets a real device badly | rework when the driver arrives | every layer is a borrowed view over bytes with no assumption about who produced them; the virtqueue logic of track F is written before the driver, not with it |
| The dual stack doubles the internet layer | track D runs long and the phases wait | the two families share the neighbor cache, the routing table, the reassembly buffers, and every crate above `net-ipv6`, so what is written twice is the header format and the address configuration and nothing else; `net-ipv6` is its own step (D4) and its own catalog item, so it can be cut back to a boundary rather than half-finished (D-69) |
| Foundations arrive after their consumers | the containers and the time arithmetic get written twice | track E is scheduled first and is small |
| The FAT32 move breaks the image writer | phase 2 tooling regresses | the move is a refactoring with the existing catalog 6.6.15 tests kept green, followed by the new tests of 6.6.52 |
| Options omitted from TCP are read as defects later | avoidable confusion | the omissions are decisions (D-50) and appear in the crate documentation |
