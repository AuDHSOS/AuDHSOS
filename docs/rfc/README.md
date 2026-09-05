# Reference documents

The standards this system implements, kept verbatim so that a vector can
be checked against its source without a network, and so that the source
cannot change under a test that cites it.

Nothing here is compiled, linked, or read at run time. These are
documents. Rule R8 of the safety policy is about dependencies, and it is
untouched: `Cargo.lock` still lists only workspace members, and no
manifest names anything outside the workspace. Decision D-59 records the
arrangement.

## What is here

| File | Document | Retrieved | Bytes | SHA-256 |
|------|----------|-----------|-------|---------|
| `rfc791.txt` | RFC 791, *Internet Protocol*, J. Postel, September 1981 | 2026-09-05 from `https://www.rfc-editor.org/rfc/rfc791.txt` | 94892 | `6cfb387fcecfc1b72f2f69343c5b6951b5d263d708162e2b5c66ab8f394f6265` |
| `rfc792.txt` | RFC 792, *Internet Control Message Protocol*, J. Postel, September 1981 | 2026-09-05 from `https://www.rfc-editor.org/rfc/rfc792.txt` | 29186 | `58714393ded142bacf188d7e8977eef98f4110c4c87ac94595f750df5664c2c6` |
| `rfc826.txt` | RFC 826, *An Ethernet Address Resolution Protocol*, D. C. Plummer, November 1982 | 2026-09-05 from `https://www.rfc-editor.org/rfc/rfc826.txt` | 21556 | `01bc62fe6a37e90f1246ac43e8e145f1322b4ed1474836145c3da93d2bd3c8a6` |
| `rfc894.txt` | RFC 894, *A Standard for the Transmission of IP Datagrams over Ethernet Networks*, C. Hornig, April 1984 | 2026-09-05 from `https://www.rfc-editor.org/rfc/rfc894.txt` | 5697 | `be88b9301e53f986aca3a0e55e488d1d79bae3f88fe3f257640397bc089e7035` |
| `rfc1071.txt` | RFC 1071, *Computing the Internet Checksum*, R. Braden, D. Borman, C. Partridge, September 1988 | 2026-09-05 from `https://www.rfc-editor.org/rfc/rfc1071.txt` | 53524 | `e10dfd6816447843d47a7f1b990eba756a791a6308fd5b698a6276075a8e4f9b` |
| `rfc1122.txt` | RFC 1122, *Requirements for Internet Hosts — Communication Layers*, R. Braden (ed.), October 1989 | 2026-09-05 from `https://www.rfc-editor.org/rfc/rfc1122.txt` | 289148 | `9f526e6bebc868324fedb90aebbcf6e5b15c53fd373ca5d5ce1c2cdcd264e04f` |
| `rfc4291.txt` | RFC 4291, *IP Version 6 Addressing Architecture*, R. Hinden, S. Deering, February 2006 | 2026-09-05 from `https://www.rfc-editor.org/rfc/rfc4291.txt` | 52897 | `4d58dff6b432d5d524bf3a3b7f0337a4177fa65f92ed72f2a92e97b471de48b2` |
| `rfc4861.txt` | RFC 4861, *Neighbor Discovery for IP version 6 (IPv6)*, T. Narten, E. Nordmark, W. Simpson, H. Soliman, September 2007 | 2026-09-05 from `https://www.rfc-editor.org/rfc/rfc4861.txt` | 235106 | `1a4309c117d765a7c0edfcb2297fc90c3c09f0fcae255f15e53b20748bcc09fb` |
| `rfc5480.txt` | RFC 5480, *Elliptic Curve Cryptography Subject Public Key Information*, S. Turner, D. Brown, K. Yiu, R. Housley, T. Polk, March 2009 | 2026-09-05 from `https://www.rfc-editor.org/rfc/rfc5480.txt` | 36209 | `593bf29fd0da2ff8b903c3ebf1c9d189a770039159e2ba46a0c3b91355037f26` |
| `rfc5758.txt` | RFC 5758, *Internet X.509 Public Key Infrastructure: Additional Algorithms and Identifiers for DSA and ECDSA*, Q. Dang, S. Santesson, K. Moriarty, D. Brown, T. Polk, January 2010 | 2026-09-05 from `https://www.rfc-editor.org/rfc/rfc5758.txt` | 15834 | `4d02628ff0875a1960d34be584a68f88528b96242bdc5a05a40a29ef01cf1532` |
| `rfc5903.txt` | RFC 5903, *Elliptic Curve Groups modulo a Prime (ECP Groups) for IKE and IKEv2*, D. Fu, J. Solinas, June 2010 | 2026-09-05 from `https://www.rfc-editor.org/rfc/rfc5903.txt` | 29175 | `939fab548a6e6bb49a5b3c4dd24a3c5df54a46645447b2d6f4df4fd88ff2d69f` |
| `rfc5952.txt` | RFC 5952, *A Recommendation for IPv6 Address Text Representation*, S. Kawamura, M. Kawashima, August 2010 | 2026-09-05 from `https://www.rfc-editor.org/rfc/rfc5952.txt` | 26570 | `c75e82c5f53bcec8148820fadf0d65935336ee2031fa6ce10504797ed4c1979d` |
| `rfc6979.txt` | RFC 6979, *Deterministic Usage of the Digital Signature Algorithm (DSA) and Elliptic Curve Digital Signature Algorithm (ECDSA)*, T. Pornin, August 2013 | 2026-09-05 from `https://www.rfc-editor.org/rfc/rfc6979.txt` | 140386 | `456e8f17558fdbd206f968b96fc6f1b4a71ea331ab30ad17f711ab3adaa7d701` |
| `rfc8200.txt` | RFC 8200, *Internet Protocol, Version 6 (IPv6) Specification*, S. Deering, R. Hinden, July 2017 | 2026-09-05 from `https://www.rfc-editor.org/rfc/rfc8200.txt` | 93162 | `371ae3f133d562db5d6385e6def4ca9914c4f831be228ea7779fd28799c2f490` |
| `rfc8448.txt` | RFC 8448, *Example Handshake Traces for TLS 1.3*, M. Thomson, January 2019 | 2026-09-05 from `https://www.rfc-editor.org/rfc/rfc8448.txt` | 159343 | `6564d1376d1ec744fc7a9993da15ebc1b9be361908b166091f47ef605c537fba` |

The checksums are here so that a reader can tell a file has not been
edited. Each is the text as the RFC Editor publishes it, byte for byte,
including the page breaks: 2887, 1218, 470, 171, 1417, 6844, 1403, 5435,
1123, 451, 899, 787, 4427, 2355, and 3811 lines respectively, in the
order of the table.
Every one was fetched twice and the two fetches agreed.

## Terms

These documents are not covered by this repository's licence. Each
carries a notice of the form

> Copyright (c) YEAR IETF Trust and the persons identified as the
> document authors. All rights reserved.

with the year 2009 for RFC 5480, 2010 for RFC 5758, RFC 5903, and
RFC 5952, 2013 for RFC 6979, 2017 for RFC 8200, and 2019 for RFC 8448.
Those eight are subject to BCP 78 and the IETF Trust's Legal Provisions
relating to IETF Documents, which permit reproduction in full.

The seven older ones carry the notice of their time. RFC 4861 has the IETF
Trust's of 2007 and RFC 4291 the Internet Society's of 2006, both in a
full copyright statement at the end that permits reproduction in full
under the same BCP 78, and RFC 1122 carries that statement in the form of
1989. RFC 1071, RFC 894, RFC 826, RFC 792, and RFC 791 carry no notice at
all: RFC 1071 states unlimited distribution in its own Status of This
Memo section, and the four from the early eighties predate even that
form, under the practice the RFC Editor states for the series as a whole.

Code components extracted from an RFC carry the Simplified BSD Licence;
this project extracts test vectors, which it transcribes into Rust source
with the document and section named at each table, as decision D-40
requires.

## Why RFC 1071

The internet checksum is one algorithm used by IP, ICMP, UDP, and TCP, and
`net-wire` computes it for all four. Section 3 of the memo is a worked
example with every intermediate value written out — the same eight bytes
summed byte by byte, as 16-bit words in both byte orders, and as 32-bit
words in three orders, followed by the same sum split into two groups
across an odd boundary. That last table is the one that checks an
accumulator which carries a pending byte from one chunk to the next, and
it is not in RFC 791, RFC 768, or RFC 9293, which state the checksum and
give no numbers for it. The vectors of catalog 6.6.42 are transcribed from
it.

The memo is not a standard; it says so itself. What is normative is that
the sum is the 16-bit one's complement of the one's complement sum, and
that comes from the protocol specifications. This document is kept for its
numbers and for section 2, which states why the sum may be computed in
either byte order and in any grouping — the property the incremental
accumulator rests on.

## The three documents of IPv4

**RFC 791** is the protocol: the twenty-byte header and its fields, the
rule that options are a whole number of words and may be skipped by a
host that does not implement them, and — section 3.2, *Fragmentation and
Reassembly* — the identification, flags, and offset that a datagram is
cut and put together by, with the reassembly procedure written out as an
algorithm.

**RFC 792** is ICMP: the echo request and reply, destination unreachable
with its codes, and time exceeded, each carrying the header of the
datagram that caused it plus eight bytes, which is what lets the upper
layer match an error to the connection that earned it.

**RFC 1122, section 3.2.2** is the list this implementation follows most
carefully: an ICMP error is never sent in answer to an ICMP error, to a
datagram addressed to a broadcast or multicast address, to one that
arrived as a link-layer broadcast, to a non-initial fragment, or to one
whose source address names no single host. The memo says these
restrictions take precedence over every other requirement to send an
error, and it explains why — a broadcast to a closed port would otherwise
draw an answer from every host on the link at once.

## The three documents of the link layer

These are what `net-eth` implements.

**RFC 894** is three pages and settles the frame: an IP datagram on an
Ethernet is carried in an Ethernet II frame with type `0x0800`, ARP with
`0x0806`, and the payload is at most 1500 bytes. It is where the MTU
comes from, and it says the trailing checksum belongs to the hardware,
which is why nothing in this crate computes one.

**RFC 826** is the address resolution protocol: the 28-byte packet for
Ethernet and IPv4 with its six fields, the rule that a reply is sent only
by the station that owns the target address, and the observation — the
one this implementation follows most closely — that a station may learn a
mapping from any packet it sees rather than only from a reply it asked
for. What the memo does not say is what to do when a second station
claims an address a first one already answered for, which is where the
project's own rule comes in: a reachable entry is never overwritten with
a different hardware address.

**RFC 4861, section 7.3** is the neighbor cache: the five states
`INCOMPLETE`, `REACHABLE`, `STALE`, `DELAY`, and `PROBE`, what moves an
entry between them, and the constants of section 10 — `REACHABLE_TIME`
30 seconds, `RETRANS_TIMER` 1 second, `DELAY_FIRST_PROBE_TIME` 5 seconds,
and three solicitations of either kind. The cache here is one cache for
both families (D-69), so ARP fills it with the three states it needs and
Neighbor Discovery, which arrives in D4, uses all five.

## The three documents of IPv6 addressing

These arrived with D-69, which put IPv6 into the first network version
beside IPv4. They are what `net-wire` implements; the protocol itself is
later work, and RFC 8200 is here already because its section 8.1 is part
of this step.

**RFC 4291** is the address architecture: the 128-bit address, the text
forms of section 2.2, the prefixes that make an address unspecified,
loopback, link-local, unique-local, or multicast, and — section 2.7.1 —
the solicited-node multicast address, which is the low 24 bits of a
unicast address appended to `ff02::1:ff00:0/104`. Neighbor Discovery is
addressed to that group rather than to a broadcast, which is the reason
`Ipv6Addr::solicited_node` exists before there is a neighbor to discover.

**RFC 5952** is the canonical text form, and the reason the address type
has one spelling per value. Section 4 is four rules: leading zeros
suppressed, `::` used to its maximum, `::` never used for a single zero
field, and on a tie the leftmost run shortened; section 4.3 requires
lower case. RFC 4291 permits several texts per address; this document
picks one of them, and the parser here refuses the others (D-69).

**RFC 8200, section 8.1** is the pseudo-header UDP, TCP, and ICMPv6
compute their checksums over: the two 128-bit addresses, a 32-bit
upper-layer length, three zero bytes, and the next-header value of the
upper-layer protocol — which is not the next-header field of the packet
when extension headers stand between them. The rest of the document is
the header format and the extension header chain, which the IPv6 step
will read.

## Why RFC 8448 in particular

It is the only published trace of a complete TLS 1.3 handshake: every
secret, every message, and every record of one connection, with the
private keys that produced them. A client that reproduces it byte for
byte has been checked against an implementation that was not this one,
which no amount of testing a client against its own idea of a server can
do.

## The four documents of P-384

These four are what P-384 took, and no more. They were gathered because
`crypto-ec` had P-256 and Ed25519 and no P-384, which is where a real
chain stopped: Google Trust Services issues from a P-256 intermediate
under a P-384 root, so `SubjectPublicKey::parse` reached `GTS Root R4`,
saw a curve that was not `prime256v1`, and answered
`UnsupportedAlgorithm`. Every anchor above such an intermediate was out of
reach. The curve is implemented now, and these are the documents its
constants and its vectors come from.

**RFC 5903, section 3.2** — the numbers. The prime
`p = 2^384 - 2^128 - 2^96 + 2^32 - 1`, the curve `y^2 = x^3 - 3x + b`
with `b` in full, the generator `g = (gx, gy)`, the group order, and the
seed the curve was verifiably generated from. Section 5, *Alignment with
Other Standards*, is the table that says the 384-bit random ECP group,
NIST P-384, and SECG `secp384r1` are one curve under three names. RFC 6090 points here for the parameter set,
which is why this is the copy that is kept.

**RFC 5480** — how the key is written down. Section 2.1.1.1 gives
`secp384r1` the identifier `1.3.132.0.34`; section 2.2 gives the
uncompressed point encoding, `0x04` followed by `x` and `y`, which for
P-384 is 97 bytes where P-256 takes 65; section 4 pairs a key of 384 bits
with SHA-384. The ASN.1 module also carries
`ECDSA-Sig-Value ::= SEQUENCE { r INTEGER, s INTEGER }`, the shape a
signature arrives in. This is the document `SubjectPublicKey::parse`
implements, and the one place it has to change.

**RFC 5758, section 3.2** — `ecdsa-with-SHA384` is
`1.2.840.10045.4.3.3`, and its `AlgorithmIdentifier` omits the parameters
field rather than encoding a null. This half was already done when the
others were not: `oid::ECDSA_WITH_SHA384` and
`SignatureAlgorithm::EcdsaSha384` existed and were exercised. The document
is kept because the identifier and the key it names belong to one change,
and a reader of the other three should not have to go looking for it.

**RFC 6979, appendix A.2.6** — the vectors. A P-384 key pair given as the
private scalar `x` and the public point `Ux, Uy`, the group order `q`,
and ten signatures: `k`, `r`, and `s` for SHA-1, SHA-224, SHA-256,
SHA-384, and SHA-512, over each of the messages `"sample"` and `"test"`.
It is the same table one curve up from the one the P-256 code is already
checked against: `crates/crypto/ec/src/tests/p256.rs` transcribes
appendix A.2.5, and `the_rfc_6979_vectors_sign_and_verify_as_documented`
is the test that reads it.

Note what the appendix does not give: it states the order `q` but not
`p`, `b`, or the generator, so a verifier tested against it is tested
against RFC 5903 as well. The two are one vector set.

## What P-384 does not need, and why it is not here

These were read and left out. They are named so that the next reader does
not repeat the search.

- **RFC 5114, section 2.7** states the same P-384 constants as RFC 5903,
  with `a` written out in hexadecimal instead of as `-3`. Comparing the
  two catches a transcription slip, but it is not a second source: both
  are transcriptions of FIPS 186 and SEC 2. Read it there if the check is
  wanted; it earns no copy.
- **RFC 3279** defines `ECDSA-Sig-Value`, but RFC 5480 updates it and
  carries the same definition in its own module.
- **RFC 6090** gives elliptic curve algorithms without naming P-384, and
  points at RFC 5903 for the parameters.
- **RFC 8422** is ECC for TLS 1.2 and earlier. This client speaks 1.3
  only.
- **RFC 9500** carries an encoded P-384 test key. This repository builds
  the certificates it tests with, in `audhsos-x509`'s builder.
- **RFC 8446** assigns `secp384r1` the group `0x0018` and
  `ecdsa_secp384r1_sha384` the scheme `0x0503`. P-384 here is for reading
  a chain, not for the key exchange, which decision D-56 settles on
  `x25519` alone. Whether the TLS 1.3 specification itself belongs in this
  directory is a separate question from this one.
