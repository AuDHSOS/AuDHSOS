# net-wire, net-eth, net-ip audit findings

Repository: AuDHSOS/AuDHSOS. Audit of net-wire, net-eth, net-ip at main bc47df5.
Each `## ` section is one issue, filed 2026-09-19 with the number in its heading. `Title:` is the issue title, `Labels:` the labels, everything after `Body:` is the issue body verbatim.

---

## F01 — issue #272
Title: net-ip: the sender reads the neighbor cache and never calls resolve, so a stale neighbor is never probed and its use is never recorded
Labels: bug, part::net
Body:
`Sender::send` takes the hardware address with `NeighborCache::hardware` (`crates/net/ip/src/send.rs:125`), which only reads the entry (`crates/net/eth/src/neighbor.rs:217-221`). `NeighborCache::resolve` is called only from `hold` when no address is known (`crates/net/ip/src/send.rs:184-197`). The Stale-to-Delay transition of RFC 4861 section 7.3.3 and the `used` timestamp live in `resolve` (`crates/net/eth/src/neighbor.rs:233-241`), so a datagram to a known neighbor triggers neither.

Sequence: the gateway is confirmed at `t = 0`; its entry goes Stale at `t = 30 s` (`crates/net/eth/src/neighbor.rs:423-433`); every later `send` delivers to the stored address, and `poll` never produces a probe, because a Stale entry has the deadline `Instant::MAX`. The gateway's replacement with another hardware address is detected only through an ARP packet the replacement sends. The same entry keeps `used = 0`, so `insert` evicts the neighbor in constant use first (`crates/net/eth/src/neighbor.rs:493-498`) when sixteen other addresses claim entries (F03).

Fix: `send` calls `self.neighbors.resolve(neighbor, payload, now)` and maps `Deliver`, `Waiting`, `Dropped` to `Sent`, which removes `hold`; the option of a separate `touch(address, now)` on the cache keeps two calls where one suffices.

---

## F02 — issue #275
Title: net-eth: on_confirmed moves a Reachable entry to any hardware address, so an unsolicited reply takes a neighbor mid-conversation
Labels: bug, part::net
Body:
`NeighborCache::on_confirmed` overwrites `hardware` of an entry in every state (`crates/net/eth/src/neighbor.rs:280-285`). The doc of `on_observed` states that a Reachable entry cannot be moved to another hardware address by an unsolicited claim and that a neighbor this host talks to "cannot be stolen mid-conversation" (`crates/net/eth/src/neighbor.rs:303-316`). The cache has no record of an outstanding solicitation, so it cannot tell a solicited reply from an unsolicited one. `net-stack` calls `on_confirmed` for every ARP packet whose target is one of this host's addresses, request or reply (`crates/net/stack/src/receive.rs:63-70`).

Trigger: the gateway entry is Reachable. An attacker sends one ARP reply with `target_protocol` = this host, `sender_protocol` = the gateway, `sender_hardware` = the attacker. The entry becomes Reachable at the attacker's address for another `reachable` period, and every datagram to the gateway goes to the attacker.

Fix: `on_confirmed` takes a different hardware address only into an entry in `Incomplete`, `Delay`, or `Probe`, where a solicitation of this host is outstanding, and behaves as `on_observed` otherwise; the option of keeping the RFC 826 merge rule requires the doc at `crates/net/eth/src/neighbor.rs:303-316` to drop the claim.

---

## F03 — issue #278
Title: net-eth: on_confirmed and on_observed create entries for unknown addresses, so unsolicited packets flush the cache
Labels: bug, part::net
Body:
`on_confirmed` creates a Reachable entry for an address not in the cache (`crates/net/eth/src/neighbor.rs:287-297`), and `on_observed` creates a Stale one (`crates/net/eth/src/neighbor.rs:335-346`). Both go through `insert`, which evicts the least recently used entry when the cache is full (`crates/net/eth/src/neighbor.rs:489-500`). RFC 826 adds a new entry only when the packet targets this host and the opcode is a request (`docs/rfc/rfc826.txt:209-218`); RFC 4861 section 7.2.5 discards an advertisement for a target without an entry (`docs/rfc/rfc4861.txt:3563-3568`). The doc states the eviction case is not reached by a cache sized for the neighbors a host talks to (`crates/net/eth/src/neighbor.rs:149-157`); the attacker chooses the number.

Trigger: with `NEIGHBORS = 16` (`crates/net/stack/src/stack.rs:41`), sixteen ARP packets from sixteen distinct sender addresses, gratuitous or addressed to this host, evict every entry, including an Incomplete entry with its held packet, which is lost without an `Unreachable` event. The packets cost the attacker 16 frames per flush.

Fix: `on_confirmed` and `on_observed` update an existing entry and return `false` for an unknown address, and a new `learn(address, hardware, now)` creates a Stale entry for the two cases the RFCs allow, a request addressed to this host and a neighbor solicitation; the option of evicting only Reachable and Stale entries keeps the flush of those.

---

## F04 — issue #281
Title: net-ip: an overlapping fragment does not discard the datagram, contrary to the module doc and the error doc
Labels: bug, part::net
Body:
`Buffer::insert` returns `IpError::OverlappingFragment` and changes nothing (`crates/net/ip/src/fragment.rs:493-503`); `accept_piece` propagates the error with `?` and the buffer stays in `buffers` (`crates/net/ip/src/fragment.rs:405-407`). The module doc states "An overlap discards the datagram" (`crates/net/ip/src/fragment.rs:12-17`) and the error doc states "the whole datagram is discarded" (`crates/net/ip/src/error.rs:37-40`). The test checks the error only (`crates/net/ip/src/tests/fragment.rs:203-230`), and `net-stack` returns on the error without `release` (`crates/net/stack/src/receive.rs:180-182`).

Sequence, run against the crate: fragment `(0, 16, MF)`, then `(8, 8, last)` with other bytes, then `(16, 8, last)`. The second returns `OverlappingFragment`; the third completes a datagram of 24 bytes from the first and the third. The datagram the doc says is gone is delivered.

Fix: `accept_piece` removes the buffer at `index` when `insert` returns `OverlappingFragment`; the option of keeping the buffer and correcting the two docs makes the first fragment win, which the module doc argues against.

---

## F05 — issue #284
Title: net-ip: a zero-length fragment is stored as a range, and 64 of them lock the datagram until its deadline
Labels: bug, part::net
Body:
`Buffer::insert` pushes `(at, end)` for every piece that touches no filled range (`crates/net/ip/src/fragment.rs:505-509`). For an empty payload `at == end`, and the touch test `at < stop && start < end` (`crates/net/ip/src/fragment.rs:493`) is false against every range, including an identical empty one, so duplicates are not detected. `filled` holds 64 ranges (`crates/net/ip/src/fragment.rs:257`), and a full list turns every later piece into `TooLarge` while the buffer stays.

Trigger, run against the crate: 64 fragments with offset 8, MF set, and no payload for one key are accepted; the 65th and every real fragment for that key, including the first at offset 0, return `TooLarge(8)` for `REASSEMBLY_TIMEOUT`. An attacker who spoofs the source of a peer blocks the peer's fragmented datagram of that identification for 60 seconds with 64 frames of 34 bytes.

Fix: `insert` returns `Ok(())` without storing a piece whose payload is empty, after recording `total` when `more` is false; the option of removing the buffer on a full `filled` list drops the datagram instead of the attacker's pieces.

---

## F06 — issue #287
Title: net-ip: a second last fragment overrides the total length, so a datagram is delivered truncated
Labels: bug, part::net
Body:
`Buffer::insert` sets `total = Some(end)` on every piece with `more == false` (`crates/net/ip/src/fragment.rs:510-512`). A piece with `more == false` and an end below a total already recorded overwrites it, and `is_complete` then compares the filled ranges against the smaller total (`crates/net/ip/src/fragment.rs:521-537`).

Sequence, run against the crate: `(0, 8, MF)`, `(16, 8, last)`, then `(8, 8, last)`. The third piece completes a datagram of 16 bytes; the 8 bytes at 16 are dropped. RFC 791 section 3.2 derives the total length from the last fragment, of which there is one (`docs/rfc/rfc791.txt:1644-1662`). A spoofed fragment truncates a peer's datagram to a prefix; a UDP length check catches the cut, a transport without one receives the prefix as whole.

Fix: `insert` returns `OverlappingFragment` when `total` is already `Some` and differs from `end` for a piece with `more == false`, and when `end` exceeds a known `total` for any piece; the option of keeping the first total silently ignores the disagreement.

---

## F07 — issue #290
Title: net-ip: the routing table compares prefixes by their given address, and a tie goes to the last route
Labels: bug, part::net
Body:
`RoutingTable::add` replaces a route only when `existing.destination == route.destination` (`crates/net/ip/src/route.rs:125-135`). `IpCidr` equality includes the host bits of the address as given (`crates/net/wire/src/addr.rs:277-288`), so `192.168.1.0/24` and `192.168.1.7/24` are two routes for one prefix. `lookup` picks the longest prefix with `max_by_key` (`crates/net/ip/src/route.rs:170-181`), which returns the last of equal maxima. The doc states that the first added wins and that a tie cannot happen through `add` (`crates/net/ip/src/route.rs:161-165`).

Sequence, run against the crate: `add(on_link(192.168.1.0/24))`, `add(via(192.168.1.7/24, 192.168.1.1))`, `len() == 2`, `lookup(192.168.1.5)` answers `Gateway(192.168.1.1)`. A router advertisement or a DHCP lease that names the prefix with host bits set shadows the on-link route instead of updating it, and on-link hosts are sent to the router.

Fix: `add` compares `network()` and `prefix_len()` of both routes, or normalizes `destination` to its network before storing and comparing; the option of changing `IpCidr` equality changes the meaning of an interface address.

---

## F08 — issue #292
Title: net-ip: fragment accepts a payload longer than 65515 bytes and writes wrapped offsets
Labels: bug, part::net
Body:
`fragment` bounds nothing but `mtu` (`crates/net/ip/src/fragment.rs:153-187`); each piece gets its own `Header::write`, whose total-length check covers the piece and not the datagram. `Header::write` reduces the offset with `unwrap_or(OFFSET_MASK) & OFFSET_MASK` (`crates/net/ip/src/header.rs:372-374`), which drops bits above the thirteen of the field.

Trigger, run against the crate: a payload of 70000 bytes over an MTU of 1500 yields 48 pieces whose offsets, read back, run 63640, 65120, 1064, 2544, 4024. The receiver sees the 46th piece at offset 1064 and rejects it as an overlap or assembles a wrong datagram. RFC 791 section 3.1 limits a datagram to 65535 bytes (`docs/rfc/rfc791.txt:1644-1662`).

Fix: `fragment` returns `IpError::TooLarge(payload.len())` when `MIN_HEADER_LEN + payload.len() > u16::MAX as usize`; the option of a check in `Header::write` on `fragment_offset` alone catches the wrap but not the cause.

---

## F09 — issue #294
Title: net-eth: a packet longer than the held size drops without starting resolution, so the neighbor is never asked for
Labels: bug, part::net
Body:
`NeighborCache::resolve` for an unknown address returns `Dropped` when `store` fails and does not insert the entry (`crates/net/eth/src/neighbor.rs:263-268`). No entry means `poll` emits no `Solicit`. The caller in `net-ip` maps this to `Sent::Dropped` (`crates/net/ip/src/send.rs:184-197`).

Trigger: `HELD = 512` (`crates/net/stack/src/stack.rs:44`). A UDP datagram of 600 bytes to an on-link host not in the cache returns `Dropped`, no ARP request goes out, and every repeat returns `Dropped` until some packet of at most 512 bytes to the same address is sent.

Fix: `resolve` inserts the Incomplete entry with `pending_len = 0` before it answers `Dropped`, so the solicitation goes out and a retransmission finds the address; the option of a larger `HELD` moves the bound without removing it.

---

## F10 — issue #297
Title: net-ip: may_answer_with_error treats redirect, source quench and parameter problem as answerable
Labels: bug, part::net
Body:
`Message::parse` maps types 4, 5, and 12 to `Message::Other` (`crates/net/ip/src/icmp.rs:178-181`), and `is_error` is true only for `DestinationUnreachable` and `TimeExceeded` (`crates/net/ip/src/icmp.rs:188-193`). `may_answer_with_error` relies on `is_error` (`crates/net/ip/src/icmp.rs:288-292`). RFC 1122 section 3.2.2 lists five error messages, Redirect, Source Quench and Parameter Problem among them (`docs/rfc/rfc1122.txt:2204-2210`), and forbids an error in answer to any of them (`docs/rfc/rfc1122.txt:2249-2252`).

Trigger: a datagram carrying ICMP type 12 with a valid checksum; `may_answer_with_error` answers `true`. No caller in the workspace generates an error for an ICMP datagram today, so the deviation is in the public predicate and not in a frame on the wire.

Fix: `is_error` answers true for `Message::Other` with `message_type` 4, 5, or 12, named as constants beside the four known types; the option of parsing the three messages adds fields nothing reads.

---

## F11 — issue #299
Title: net-ip: the source test of may_answer_with_error misses the directed broadcast and class E
Labels: bug, part::net
Body:
The source predicate refuses the unspecified, limited broadcast, loopback and multicast addresses (`crates/net/ip/src/icmp.rs:302-306`). RFC 1122 section 3.2.2 names "a broadcast address" and "a Class E address" (`docs/rfc/rfc1122.txt:2261-2264`). The function receives `interface_broadcast` and applies it to the destination only (`crates/net/ip/src/icmp.rs:293-297`).

Trigger: a UDP datagram from source `192.168.1.255` on a `/24` interface to a closed port. The predicate answers `true`, the port unreachable goes to the directed broadcast address, and the neighbor cache holds it as Incomplete for an address nobody answers for, three solicitations long (F03 makes the entry cost real). A source in `240.0.0.0/4` passes the same way.

Fix: the predicate also refuses `source == interface_broadcast` and `source.octets()[0] & 0xF0 == 0xF0`; the option of an `Ipv4Addr::is_reserved` in `net-wire` is the same test one crate lower.

---

## F12 — issue #302
Title: net-eth: the neighbor cache stores a multicast or unspecified hardware address
Labels: bug, part::net
Body:
`on_confirmed` and `on_observed` store `hardware` as given (`crates/net/eth/src/neighbor.rs:281`, `crates/net/eth/src/neighbor.rs:329`), and `Packet::parse` checks the four header fields only (`crates/net/eth/src/arp.rs:136-157`). `MacAddr::is_multicast` and `is_unspecified` exist (`crates/net/wire/src/addr.rs:56-66`) and no caller between the frame and the cache applies them.

Trigger: an ARP reply for the gateway with `sender_hardware = ff:ff:ff:ff:ff:ff` addressed to this host (F02 path, `crates/net/stack/src/receive.rs:63-70`). Every datagram to the gateway leaves as a link-layer broadcast and reaches every station, with no need for the attacker to forward anything. With `00:00:00:00:00:00` the datagrams reach nothing.

Fix: `on_confirmed` and `on_observed` return without change when `hardware.is_multicast() || hardware.is_unspecified()`, which covers ARP and Neighbor Discovery in one place; the option of a check in `Packet::parse` leaves the ND path open.

---

## F13 — issue #304
Title: net-eth: the neighbor cache keys an entry by an address that names no host
Labels: bug, part::net
Body:
Neither `on_confirmed` nor `on_observed` tests `address` (`crates/net/eth/src/neighbor.rs:278-347`). `net-stack` passes `sender_protocol` of every ARP packet (`crates/net/stack/src/receive.rs:63-74`). An ARP probe carries `sender_protocol = 0.0.0.0` (RFC 5227 is not in `docs/rfc/`; the value is what RFC 826 leaves to the sender), and a spoofed packet carries any multicast or broadcast address.

Trigger: an ARP probe for one of this host's addresses creates a Reachable entry for `0.0.0.0` (`crates/net/eth/src/neighbor.rs:287-297`); nothing is ever sent to it, and it takes one of sixteen entries until eviction.

Fix: `on_confirmed` and `on_observed` return without change when `address.is_unspecified() || address.is_multicast()`, or for `IpAddr::V4` also `is_broadcast()`; the option of the test in `net-stack` leaves the cache open to `net-ipv6`.

---

## F14 — issue #307
Title: net-eth: on_observed restarts the reachable timer on an unsolicited claim that agrees
Labels: bug, part::net
Body:
`on_observed` sets `deadline = now + reachable` for a Reachable entry whose hardware address matches (`crates/net/eth/src/neighbor.rs:322-327`). RFC 4861 section 7.2.5 states that on an advertisement with the Solicited flag zero and no address change "the entry's state remains unchanged" (`docs/rfc/rfc4861.txt:3626-3631`); the timer is part of the state. The doc at `crates/net/eth/src/neighbor.rs:306-308` says an unsolicited claim is not evidence, and the same function takes it as evidence of reachability.

Trigger: a neighbor stops answering at `t = 10 s`; a station sends unsolicited advertisements with the neighbor's address every 20 seconds. The entry stays Reachable indefinitely, and neighbor unreachability detection never runs. `net-ipv6` routes every unsolicited advertisement with the override bit here (`crates/net/ipv6/src/ndp.rs:687-691`).

Fix: the Reachable branch returns `false` without touching `deadline`; the option of keeping the refresh needs the doc at `crates/net/eth/src/neighbor.rs:303-308` to say the timer restarts.

---

## F15 — issue #310
Title: net-ip: the reassembler reports expired datagrams as a count, so the Time Exceeded of RFC 1122 cannot be sent
Labels: bug, part::net
Body:
`Reassembler::poll` removes expired buffers and returns how many (`crates/net/ip/src/fragment.rs:335-350`). RFC 1122 section 3.3.2 requires that on the reassembly timeout the datagram is discarded and an ICMP Time Exceeded message is sent when fragment zero has arrived (`docs/rfc/rfc1122.txt:3337-3342`); the requirements table marks it MUST (`docs/rfc/rfc1122.txt:4385`). `Message::TimeExceeded` carries code 1 for this case (`crates/net/ip/src/icmp.rs:127-133`) and nothing produces it.

Trigger: fragment `(0, 8, MF)` of any datagram and nothing more; sixty seconds later the buffer goes and the sender learns nothing.

Fix: `poll` takes a closure `FnMut(Piece<'_>, &[u8])` or returns the key and the first quoted bytes of each expired buffer whose `filled` covers offset 0, so the caller can write a Time Exceeded; the option of a count keeps the rule unmet.

---

## F16 — issue #314
Title: net-wire: the doc of Ipv4Cidr::parse names the wrong error for a prefix that is not decimal
Labels: bug, part::net
Body:
The doc states `WireError::PrefixLength` for a part after the slash that is "a decimal number above 255 or not decimal at all" (`crates/net/wire/src/addr.rs:349-357`). `decimal_prefix` returns `WireError::Address` in both cases (`crates/net/wire/src/addr.rs:253-268`). The doc of `Ipv6Cidr::parse` states `Address` for the same inputs (`crates/net/wire/src/addr.rs:723-730`).

Trigger: `Ipv4Cidr::parse("10.0.0.0/x")` returns `Err(WireError::Address)`; a caller matching on `PrefixLength` as documented misses it.

Fix: the doc of `Ipv4Cidr::parse` reads as the one of `Ipv6Cidr::parse`; the option of returning `PrefixLength` needs a `u8` the text does not yield.

---

## F17 — issue #316
Title: net-wire: WireError::PrefixLength is documented and displayed as an IPv4 error and carries IPv6 prefixes
Labels: bug, part::net
Body:
The variant doc states "A prefix length no IPv4 network has" (`crates/net/wire/src/error.rs:25-27`) and `Display` writes "an IPv4 prefix is at most 32 bits, not {length}" (`crates/net/wire/src/error.rs:44-46`). `Ipv6Cidr::new` returns the same variant for a length above 128 (`crates/net/wire/src/addr.rs:686-694`), and the test asserts `PrefixLength(129)` (`crates/net/wire/src/tests/ipv6.rs:164`).

Trigger: `Ipv6Cidr::parse("2001:db8::/129")` displays as "an IPv4 prefix is at most 32 bits, not 129".

Fix: the variant carries the family, `PrefixLength { version: IpVersion, length: u8 }`, and `Display` names the limit of that family; the option of a neutral text "a prefix of {length} bits is longer than the address" keeps the variant shape.

---

## F18 — issue #318
Title: net-ip: a non-last fragment whose length is not a multiple of eight is accepted and holds a slot it can never complete
Labels: enhancement, part::net
Body:
`Buffer::insert` and `accept_piece` check `end > BYTES` and overlap only (`crates/net/ip/src/fragment.rs:399-407`, `crates/net/ip/src/fragment.rs:491-514`). RFC 791 section 2.3 requires every portion but the last to be a multiple of 8 octets (`docs/rfc/rfc791.txt:695-697`). A piece `(0, 5, MF)` leaves bytes 5 to 7 unfillable, since every later offset is a multiple of eight, and the buffer waits the full `REASSEMBLY_TIMEOUT`.

The slot cost equals that of any incomplete datagram, so the bound holds today; the check frees the slot at once and refuses a shape no sender produces.

Fix: `accept_piece` returns `IpError::OverlappingFragment` or a new `IpError::Fragment` when `piece.more && piece.payload.len() % FRAGMENT_UNIT != 0`; the option of leaving it costs one slot for 60 seconds per such piece.

---

## F19 — issue #320
Title: net-wire: add_word and the pseudo-header helpers bypass the pending odd byte
Labels: enhancement, part::net
Body:
`Checksum::add_bytes` holds an odd trailing byte in `pending` (`crates/net/wire/src/checksum.rs:84-100`). `add_word` adds its word without consulting `pending` (`crates/net/wire/src/checksum.rs:70-79`), and `add_pseudo_header_v4` and `add_pseudo_header_v6` call `add_word` for the protocol and length (`crates/net/wire/src/checksum.rs:106-140`). The struct doc says a caller keeps the value between calls "including the odd byte one call ended on" (`crates/net/wire/src/checksum.rs:44-56`).

Trigger: `add_bytes(&[1])`, then `add_word(0x0203)`, then `add_bytes(&[4])` sums the words `0x0203`, `0x0104`; one call over `[1, 2, 3, 4]` sums `0x0102`, `0x0304`. No caller in the workspace mixes the two, so no wire checksum is wrong today.

Fix: `add_word` writes its two bytes through `add_bytes`, which makes the pending byte pair with the high byte of the word; the option of documenting "add_word after an even number of bytes only" leaves the trap.

---

## F20 — issue #323
Title: net-ip: slot_for builds the whole reassembly buffer on the stack before pushing it
Labels: enhancement, part::net
Body:
`slot_for` constructs `Buffer { data: [0; BYTES], filled, .. }` as a local and pushes it (`crates/net/ip/src/fragment.rs:454-461`). A `Buffer<BYTES>` holds `BYTES` plus the 64 ranges of `filled` (`crates/net/ip/src/fragment.rs:246-260`), which is about 3 KiB at the `REASSEMBLY = 2048` of `net-stack` (`crates/net/stack/src/stack.rs:50`) and 66 KiB for a reassembler sized to the 65535 of RFC 791.

The frame is inside the receive path of the server thread; the cost is one copy of `BYTES` per new datagram, O(BYTES), on every first fragment.

Fix: `ArrayVec` gains a `push_with(impl FnOnce(&mut MaybeUninit<T>))` or the buffer is reset in place with `key`, `deadline`, `filled.clear()`, `total = None` after an eviction, and only `data` is left as it was, since `filled` bounds what is read back; the option of a smaller `BYTES` is what `net-stack` does.

---

## F21 — issue #325
Title: net-ip: Route::via accepts a gateway of the other family
Labels: enhancement, part::net
Body:
`Route::via` and `RoutingTable::add` do not compare the family of `destination` and `gateway` (`crates/net/ip/src/route.rs:71-76`, `crates/net/ip/src/route.rs:125-135`). `lookup` returns `NextHop::Gateway` with the address as stored (`crates/net/ip/src/route.rs:170-181`).

Trigger: `add(via(IpCidr::V4(0.0.0.0/0), IpAddr::V6(fe80::1)))`, then `Sender::send` to an IPv4 destination resolves `fe80::1` in the neighbor cache and writes an IPv4 frame to that hardware address. The route comes from configuration, so the input is the operator's and not the network's.

Fix: `add` returns `IpError::Wire(WireError::MixedFamilies)` when `route.gateway` is `Some` of the other family; the option of a check in `via` cannot return an error from a `const fn` without changing its signature.

---

## F22 — issue #327
Title: net-eth: Frame::write emits frames below the Ethernet minimum and nothing in the workspace pads them
Labels: enhancement, part::net
Body:
`Frame::write` writes the header and the payload as given (`crates/net/eth/src/frame.rs:123-145`). RFC 894 states that the data field is at least 46 octets and is padded with zeros when necessary (`docs/rfc/rfc894.txt:30-34`). `net-stack` `push_frame` and the virtio-net driver add no padding (`crates/net/stack/src/stack.rs:537-559`; no match for padding in `crates/drivers/virtio-net/src/`).

Trigger: an ARP reply is 28 bytes of payload, a 42-byte frame. A virtio device in QEMU accepts it; a device that does not pad in hardware emits a runt frame that a switch discards.

Fix: `Frame::write` pads the payload with zeros to `MIN_PAYLOAD_LEN = 46` and `receive` keeps trimming by the total length of the layer above; the option of padding in the driver leaves every other device to do it again.
