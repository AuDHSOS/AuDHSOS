# net-ipv6 audit findings

Repository: AuDHSOS/AuDHSOS. Audit of net-ipv6 at main bc47df5.
Each `## ` section is one issue, filed 2026-09-19 with the number in its heading. `Title:` is the issue title, `Labels:` the labels, everything after `Body:` is the issue body verbatim.

---

## F01 — issue #326
Title: net-ipv6: the chain walk continues past the fragment header, so a fragment's data is read as headers and the first fragment loses its headers
Labels: bug, part::net
Body:
`Packet::upper_layer` at `crates/net/ipv6/src/header.rs:223-275` keeps stepping over extension headers after it has read a fragment header, whatever the fragment's offset and more bit say. `piece_of` at `crates/net/ipv6/src/fragment.rs:39-50` hands `upper.payload`, the bytes behind the last header the walk stepped over, to the reassembler as the piece. RFC 8200, section 4.5 (`docs/rfc/rfc8200.txt:1128-1137`) constructs the Fragmentable Part "from the fragments following the Fragment headers in each of the fragment packets", and the Next Header field of a non-first fragment's fragment header names the first header of the original packet's Fragmentable Part, not a header present in that fragment.

A conforming sender fragments a packet whose Fragmentable Part begins with a Destination Options header: fragment one carries `Fragment(next 60, offset 0, more) → DestinationOptions(8 bytes, next TCP) → TCP`, fragment two carries `Fragment(next 60, offset 8n) → data`. On fragment one the walk steps over the Destination Options header, so the piece at offset 0 starts 8 bytes late and the 8 header bytes are missing from the reassembled datagram. On fragment two the walk reads the first two data bytes as a Destination Options header, skips `(data[1] + 1) * 8` bytes of data, and sets `upper.protocol` from `data[0]`; a data byte of 44 or 43 or 0 makes the walk read further fabricated headers, and `Ipv6Error::ExtensionLength` or `Ipv6Error::ChainTooLong` drops the fragment. The stack at `crates/net/stack/src/receive.rs:259-270` dispatches the reassembled bytes on `upper.protocol` of the piece that completed the datagram, so the transport receives bytes shifted by the header length and attributed to a protocol chosen by data bytes. A datagram with a second extension header after the fragment header, which RFC 8200, section 4.1 allows, cannot be received.

Fix: end the walk at a fragment header whose offset is not zero or whose more bit is set, take the piece from the bytes directly behind that header, and after reassembly walk the reassembled bytes once more starting from the first fragment's fragment-header Next Header value; the option of continuing the walk on the first fragment only and copying the stepped-over headers into the piece leaves the non-first fragment's Next Header value with nothing to dispatch on.

---

## F02 — issue #329
Title: net-ipv6: a prefix option with the autonomous flag clear removes the address this host formed, in one packet
Labels: bug, part::net
Body:
`Configuration::learn_prefix` at `crates/net/ipv6/src/slaac.rs:372-386` finds the entry for an already known prefix and assigns `existing.address = address` and `existing.on_link = information.on_link` from the new option; `address` is `None` whenever the option's autonomous flag is clear or its prefix length is not 64 (`crates/net/ipv6/src/slaac.rs:369-370`). RFC 4862, section 5.5.3 (a) (`docs/rfc/rfc4862.txt:984-985`) has a host silently ignore a Prefix Information option whose Autonomous flag is not set, and RFC 4861, section 4.6.2 (`docs/rfc/rfc4861.txt:1651-1654`) has a host with the L flag clear "MUST NOT update a previous indication that the address is on-link".

An on-link sender emits one router advertisement with hop limit 255 carrying the host's prefix with A clear and any lifetimes. `Configuration::address()` at `crates/net/ipv6/src/slaac.rs:232-234` answers `None` from then on, and the stack's next `source_for` finds no global address. The two-hour floor of `held_valid_until` at `crates/net/ipv6/src/slaac.rs:116-128` is computed and stored on the same entry but protects nothing, because the address is already gone. The same packet with L clear sets `on_link` to false, after which `install` at `crates/net/ipv6/src/slaac.rs:510-512` stops re-adding the on-link route while the old route stays in the table. The crate documentation at `crates/net/ipv6/src/slaac.rs:271-278` and `docs/12-parallel-work.md:511` state that a router advertisement cannot expire an address this host holds; the code contradicts both.

Fix: for an existing entry, update `address` only when the option's autonomous flag is set and the prefix length is 64, and update `on_link` only when the L flag is set, leaving both fields untouched otherwise; the option of treating an A-clear option as a withdrawal through `held_valid_until` is not what RFC 4862 says and still ends an address on one packet after two hours.

---

## F03 — issue #333
Title: net-ipv6: a neighbor advertisement for a target not in the cache creates an entry, so unsolicited advertisements evict real neighbors
Labels: bug, part::net
Body:
`ndp::on_advertisement` at `crates/net/ipv6/src/ndp.rs:687-691` calls `NeighborCache::on_confirmed` for a solicited advertisement and `NeighborCache::on_observed` otherwise, without asking whether the target has an entry. `on_confirmed` at `crates/net/eth/src/neighbor.rs:278-300` and `on_observed` at `crates/net/eth/src/neighbor.rs:319-350` both insert a new entry when the address is unknown, and `insert` at `crates/net/eth/src/neighbor.rs:489-500` evicts the least recently used entry when the cache is full. RFC 4861, section 7.2.5 (`docs/rfc/rfc4861.txt:3564-3569`) has an advertisement whose target has no cache entry silently discarded: "There is no need to create an entry if none exists".

An on-link sender emits `ENTRIES + 1` neighbor advertisements to `ff02::1` with hop limit 255, distinct target addresses, and a Target Link-Layer Address option; the stack at `crates/net/stack/src/receive.rs:316-318` passes each to `on_advertisement`, and the default router's entry is evicted, so the next packet to it waits for a new resolution or is dropped when the held-packet slot is full. With the solicited flag set the same sender inserts a `Reachable` entry for the router's address under its own hardware address before the host has solicited anything, and the host sends to that address until the reachable timer runs out.

Fix: in `on_advertisement`, return `false` when `cache.state(address)` is `None` before touching the cache; the option of adding the rule to `NeighborCache` itself would also refuse the gratuitous ARP that `on_observed` exists for.

---

## F04 — issue #338
Title: net-ipv6: a router advertisement with lifetime zero from any address removes the current default router
Labels: bug, part::net
Body:
`Configuration::on_advertisement` at `crates/net/ipv6/src/slaac.rs:313-323` sets `self.router = None` whenever the advertisement's Router Lifetime is zero, without comparing `from` with the address of the router it holds. RFC 4861, section 6.3.4 (`docs/rfc/rfc4861.txt:2946-2960`) creates or times out a Default Router List entry only for the advertisement's own source address; an advertisement with lifetime zero from an address that is not in the list is ignored.

A link with two routers where router B advertises prefixes but is not a default router sends its advertisement with Router Lifetime zero, as RFC 4861, section 4.2 provides for. Every advertisement from B removes router A from `self.router`, `install` at `crates/net/ipv6/src/slaac.rs:504-509` removes the default route, and the host has no default route until A's next periodic advertisement. An on-link sender achieves the same with one packet from any link-local address, without spoofing A.

Fix: set `self.router = None` on a zero lifetime only when `from` equals `self.router.address`, and leave the router untouched otherwise; the option of a default router list with several entries is what RFC 4861 describes but is a larger change than the single-router design of the crate needs.

---

## F05 — issue #342
Title: net-ipv6: a router advertisement from a non-link-local source is accepted
Labels: bug, part::net
Body:
`ndp::receive` at `crates/net/ipv6/src/ndp.rs:235-244` checks the hop limit, the checksum, and the code, and `Configuration::on_advertisement` at `crates/net/ipv6/src/slaac.rs:297-323` stores `from` as the default router without checking its scope. RFC 4861, section 6.1.2 (`docs/rfc/rfc4861.txt:2172-2180`) has a node silently discard a router advertisement whose IP Source Address is not a link-local address. The field documentation at `crates/net/ipv6/src/slaac.rs:133-135` states the router address "is the link-local address the advertisement came from"; the code stores any address.

A router advertisement with hop limit 255 from a global or unspecified source passes `receive`, and `install` at `crates/net/ipv6/src/slaac.rs:504-506` adds a default route through that source. With the unspecified source, the route's next hop is `::`, which the neighbor cache resolves to nothing, and every off-link packet waits in the cache until the pending slot is dropped.

Fix: return `Ipv6Error::NotAdvertisement` from `Configuration::on_advertisement` when `from.is_link_local()` is false; the option of checking in `ndp::receive` covers only the advertisement type and keeps the check away from the function that stores the address.

---

## F06 — issue #346
Title: net-ipv6: the address checks of RFC 4861 sections 7.1.1 and 7.1.2 are not made on neighbor solicitations and advertisements
Labels: bug, part::net
Body:
`Discovery::parse` at `crates/net/ipv6/src/ndp.rs:138-191` and `ndp::receive` at `crates/net/ipv6/src/ndp.rs:235-244` check the hop limit, the checksum, the code, the fixed length, and the option lengths. RFC 4861, section 7.1.1 (`docs/rfc/rfc4861.txt:3286-3294`) also requires of a solicitation that the target is not a multicast address, that an unspecified source comes with a solicited-node multicast destination, and that an unspecified source comes with no source link-layer address option; section 7.1.2 (`docs/rfc/rfc4861.txt:3333-3336`) requires of an advertisement that the target is not a multicast address and that a multicast destination comes with the solicited flag clear. The module documentation at `crates/net/ipv6/src/ndp.rs:26-29` states that `receive` checks "all three" requirements of section 7.1, naming a subset of the section's list.

An advertisement to `ff02::1` with the solicited and override flags set and a Target Link-Layer Address option reaches `on_advertisement` at `crates/net/ipv6/src/ndp.rs:687-690`, which calls `on_confirmed` and marks the entry `Reachable` under the advertised hardware address without any solicitation from this host. An advertisement whose target is a multicast address inserts a cache entry for that multicast address. A solicitation from `::` sent to a unicast address of this host, which a conforming receiver discards, is taken by `Dad::on_solicitation` at `crates/net/ipv6/src/slaac.rs:664-674` as a competing duplicate address detection and ends the check with `Duplicate`.

Fix: give `receive` the checks it lacks, using `packet.source()` and `packet.destination()`, and return `Ipv6Error::NotDiscovery` for a message that fails one; the option of checking in the stack's `on_discovery` leaves `receive`'s documentation false.

---

## F07 — issue #348
Title: net-ipv6: a routing header with segments left is stepped over and the packet is delivered
Labels: bug, part::net
Body:
`Packet::upper_layer` at `crates/net/ipv6/src/header.rs:254-261` reads a routing header for its length only; the documentation at `crates/net/ipv6/src/header.rs:204-207` says so. RFC 8200, section 4.4 (`docs/rfc/rfc8200.txt:796-802`) has a node ignore a routing header of an unrecognized type only when Segments Left is zero, and discard the packet with an ICMP Parameter Problem, code 0, when Segments Left is non-zero. This host recognizes no routing type, so every routing header is of an unrecognized type; RFC 5095 deprecates type 0 with the same behavior.

A packet addressed to this host with a routing header of type 0 and Segments Left 1 names this host as an intermediate hop and another address as the final destination. The walk delivers the payload to the transport as if this host were the final destination, and a TCP segment or UDP datagram in it is accepted by a socket. A conforming host discards the packet.

Fix: in the routing branch, read the third byte of the header (Segments Left) and return an error, for example `Ipv6Error::ExtensionLength`, when it is not zero; the option of also sending the Parameter Problem message needs a message writer the crate does not have.

---

## F08 — issue #351
Title: net-ipv6: a hop-by-hop options header that is not first in the chain is accepted
Labels: bug, part::net
Body:
`Packet::upper_layer` at `crates/net/ipv6/src/header.rs:223-275` accepts `Protocol::HOP_BY_HOP` as the next header of any extension header. RFC 8200, section 4 (`docs/rfc/rfc8200.txt:434-436` and `docs/rfc/rfc8200.txt:471-473`) has the Hop-by-Hop Options header immediately follow the IPv6 header when present, and has a node that encounters a Next Header value of zero in any header other than the IPv6 header discard the packet and send an ICMP Parameter Problem, code 1.

A packet with `DestinationOptions → HopByHop → UDP` is walked to the UDP header and delivered. The test at `crates/net/ipv6/src/tests/header.rs:194` states that a repeated header is stepped over like any other, so the accepted chain is the tested behavior.

Fix: in the loop, return an error when `protocol == Protocol::HOP_BY_HOP` and `headers > 1`; the option of tracking the full recommended order of RFC 8200, section 4.1 is a recommendation, where the hop-by-hop position is a requirement.

---

## F09 — issue #353
Title: net-ipv6: options inside hop-by-hop and destination options headers are not read, so an option whose type requires a discard is accepted
Labels: bug, part::net
Body:
`Packet::upper_layer` at `crates/net/ipv6/src/header.rs:254-261` steps over a Hop-by-Hop Options or Destination Options header by its length field and reads none of its options; the documentation at `crates/net/ipv6/src/header.rs:204-207` says "none is interpreted". RFC 8200, section 4.2 (`docs/rfc/rfc8200.txt:592-608`) encodes in the two highest bits of an Option Type the action a node that does not recognize the option must take: `01`, `10`, and `11` discard the packet, and `10` and `11` also send an ICMP Parameter Problem, code 2. Section 4.6 has the Destination Options header examined by the destination node.

A packet with a Destination Options header carrying one option of type `0xC0` (bits `11`, unrecognized) followed by a UDP header is delivered to the socket; a conforming destination discards it. An option whose own length field runs past its header is not detected either, since only the header's length field is read.

Fix: walk the options of the two headers by type and length, skip types 0 and 1 and every type whose top two bits are `00`, and return an error for any other type or for an option length that runs past the header; the option of leaving the hop-by-hop header unread is what RFC 8200 allows for nodes en route, not for the destination's Destination Options header.

---

## F10 — issue #356
Title: net-ipv6: an unsolicited advertisement with the override flag set does not replace the hardware address of a reachable entry
Labels: bug, part::net
Body:
`ndp::on_advertisement` at `crates/net/ipv6/src/ndp.rs:680-691` calls `NeighborCache::on_observed` for an unsolicited advertisement whether or not its override flag is set, and `on_observed` at `crates/net/eth/src/neighbor.rs:321-327` returns without changes when the entry is `Reachable` and the hardware address differs. RFC 4861, section 7.2.5 II (`docs/rfc/rfc4861.txt:3617-3630`) has an advertisement with the Override flag set update the cached link-layer address and set the entry to `Stale` when the advertisement is unsolicited. The documentation at `crates/net/ipv6/src/ndp.rs:650-657` states that the override bit is honored and describes only the case where it is clear.

A neighbor whose interface card was replaced announces its new address the way RFC 4861, section 7.2.6 (`docs/rfc/rfc4861.txt:3672-3684`) prescribes: an unsolicited advertisement to `ff02::1` with the Override flag set. While this host's entry for it is `Reachable`, the announcement changes nothing, and this host sends to the old hardware address until the reachable timer runs out, the entry passes through `Stale`, `Delay`, and `Probe` against the old address, and the entry is dropped and re-resolved.

Fix: when `overriding` is set and the advertisement is unsolicited, call a cache operation that replaces the hardware address and sets the entry `Stale` in every state, and keep `on_observed` for the case without the flag; the option of leaving the behavior as a spoofing defense is not what the documentation claims and costs a conforming neighbor a reachable time of outage.

---

## F11 — issue #357
Title: net-ipv6: a full prefix or server table ends the processing of the advertisement and the stack discards what was learned
Labels: bug, part::net
Body:
`Configuration::on_advertisement` at `crates/net/ipv6/src/slaac.rs:329-350` returns the first error from `learn_prefix` or `learn_server`, and both return `Ipv6Error::Ip(IpError::NoRoute)` when their `ArrayVec` is full (`crates/net/ipv6/src/slaac.rs:393-401` and `crates/net/ipv6/src/slaac.rs:430-435`). The options behind the one that failed are not read, and the stack at `crates/net/stack/src/receive.rs:328-338` calls `configure_from_advertisement` only when the result is `Ok`, so the router, hop limit, MTU, and prefixes the same advertisement carried before the failing option are stored in the configuration and not installed.

An on-link sender emits one advertisement with an RDNSS option naming `SERVERS` addresses and infinite lifetime. The server table is full from then on and never expires. Every later advertisement of the real router that carries an RDNSS option with a new address fails at `learn_server`, and when that option precedes the prefix option the prefix is not learned. An RDNSS option of the maximum length names 127 addresses, so one option is enough for any `SERVERS`. RFC 8106, section 5.3.1 (`docs/rfc/rfc8106.txt:476-480`) leaves the number of addresses to keep to local policy and does not have the host stop reading the advertisement.

Fix: make a full table drop the entry that did not fit and continue with the next option, returning `Ok` and counting the drop, which also removes the misleading `IpError::NoRoute` variant that the documentation at `crates/net/ipv6/src/slaac.rs:291-296` has to explain; the option of evicting the server with the nearest expiry keeps the table under a sender's control.

---

## F12 — issue #359
Title: net-ipv6: two prefix options for one prefix are two entries when their bits after the prefix length differ
Labels: bug, part::net
Body:
`Configuration::learn_prefix` at `crates/net/ipv6/src/slaac.rs:366-375` builds `Ipv6Cidr::new(information.prefix, information.prefix_len)` and finds an existing entry by `existing.prefix == prefix`. `Ipv6Cidr` at `crates/net/wire/src/addr.rs:668-693` derives `PartialEq` over the address as given and does not mask it. RFC 4861, section 4.6.2 (`docs/rfc/rfc4861.txt:1697-1700`) has the bits after the prefix length "ignored by the receiver", and RFC 4862, section 5.5.3 (d) (`docs/rfc/rfc4862.txt:994-998`) defines two prefixes as equal when their first prefix-length bits are identical.

Two advertisements for `2001:db8::/64` whose option bodies differ in byte 24 create two `Configured` entries with the same address (`address_from` at `crates/net/ipv6/src/slaac.rs:81-87` reads the first eight bytes only) and two on-link routes through `install` at `crates/net/ipv6/src/slaac.rs:510-512`, since `RoutingTable::add` at `crates/net/ip/src/route.rs:125-135` compares the same unmasked value. The second entry consumes a `PREFIXES` slot, and the lifetime refresh of one advertisement does not reach the entry the other created, so the entry the first advertisement created expires and hands `Expired::Prefix` to the caller, which removes the on-link route while the second entry still lists the prefix.

Fix: build the entry from `Ipv6Cidr::new(prefix.network(), prefix_len)` so that the stored and compared value is the masked prefix; the option of comparing with `Ipv6Cidr::contains` in both directions stores an unmasked value that the routing table then compares unmasked.

---

## F13 — issue #361
Title: net-ipv6: a prefix option whose prefix is a multicast address forms a source address from it
Labels: bug, part::net
Body:
`Configuration::learn_prefix` at `crates/net/ipv6/src/slaac.rs:361-370` rejects a link-local prefix and forms an address from every other prefix of length 64 with the autonomous flag set. RFC 4291, section 2.7 (`docs/rfc/rfc4291.txt:815-816`) has multicast addresses never used as source addresses.

An advertisement carrying the prefix `ff02::/64` with the autonomous flag set forms `ff02::<interface identifier>`, `Configuration::address()` at `crates/net/ipv6/src/slaac.rs:232-234` answers it, the stack at `crates/net/stack/src/drive.rs:231-239` runs duplicate address detection for it, and after that check every packet the host sends from its global address carries a multicast source, which every receiver discards. The unspecified prefix `::/64` forms an address in `::/64` the same way.

Fix: return early from `learn_prefix` when `information.prefix.is_multicast()` or the prefix is the unspecified prefix; the option of checking in the stack leaves `Configuration::address()` answering an address the crate's own `may_answer_with_error` refuses as a source.

---

## F14 — issue #363
Title: net-ipv6: a payload longer than the fragment offset field can name is cut into fragments with a clamped offset
Labels: bug, part::net
Body:
`emit_frames` at `crates/net/ipv6/src/send.rs:237-266` cuts any payload length through `Fragments::with_header` at `crates/net/ip/src/fragment.rs:63-86`, which bounds nothing, and writes each piece's offset through `FragmentHeader::write` at `crates/net/ipv6/src/header.rs:421-422`, where `u16::try_from(self.offset).unwrap_or(FRAGMENT_OFFSET_MASK)` replaces an offset above 65535 with 65528. RFC 8200, section 4.5 (`docs/rfc/rfc8200.txt:847`) gives the Fragment Offset field thirteen bits of eight-byte units, so the largest offset a fragment can carry is 65528 and the largest Fragmentable Part is 65535 bytes.

A caller passes a payload of 70000 bytes to `Sender::send`. The fragments at offsets 65536 and above go out with offset 65528, the receiver sees overlapping pieces and discards the datagram, and `send` answers `Ok(Sent::Frames(n))` as if every piece were correct.

Fix: return `Ipv6Error::PayloadTooLong(payload.len())` from `emit_frames` when `payload.len()` exceeds 65535, and make `FragmentHeader::write` return an error instead of clamping; the option of bounding in `Fragments::with_header` changes the shared `net-ip` API for an IPv6-only limit.

---

## F15 — issue #365
Title: net-ipv6: a buffer shorter than the path MTU is reported as WouldFragment
Labels: bug, part::net
Body:
`emit_frames` at `crates/net/ipv6/src/send.rs:217-220` takes `buffer.get_mut(..mtu)` and answers `Ipv6Error::WouldFragment { length, mtu }` when the caller's buffer is shorter than the MTU. The variant's documentation at `crates/net/ipv6/src/error.rs:80-87` describes a packet that does not fit the MTU, and the `send` documentation at `crates/net/ipv6/src/send.rs:96-99` lists that meaning only.

A caller with a 1280-byte buffer and a path MTU of 1500 sends a 100-byte payload and receives `WouldFragment { length: 100, mtu: 1500 }`, an error whose text says 100 bytes do not fit 1500.

Fix: answer `Ipv6Error::Wire(WireError::OutOfBounds { needed: mtu, available: buffer.len() })` for the short buffer; the option of cutting the packet to `buffer.len()` sends smaller fragments than the path allows without the caller knowing.

---

## F16 — issue #367
Title: net-ipv6: the hop limit a router advertises is stored and never used by the sender
Labels: enhancement, part::net
Body:
`Configuration::on_advertisement` at `crates/net/ipv6/src/slaac.rs:324-326` stores the advertised Cur Hop Limit, and `Configuration::hop_limit` at `crates/net/ipv6/src/slaac.rs:241-249` answers it. `Outgoing` at `crates/net/ipv6/src/send.rs:45-56` has no hop limit field, and `emit_frames` at `crates/net/ipv6/src/send.rs:224-229` and `crates/net/ipv6/src/send.rs:244-252` builds every header through `Header::new`, which sets `DEFAULT_HOP_LIMIT` at `crates/net/ipv6/src/header.rs:353`. RFC 4861, section 6.3.4 (`docs/rfc/rfc4861.txt:2978-2980`) has a host set its CurHopLimit to a non-zero advertised value.

A router that advertises a hop limit of 32, or of 128, for its link has no effect on what this host sends; the stored value has no reader in the workspace.

Fix: add `hop_limit: u8` to `Outgoing`, pass it through to `Header`, and have the stack fill it from `Configuration::hop_limit().unwrap_or(DEFAULT_HOP_LIMIT)`; the option of removing the stored value drops a field the advertisement is documented to carry.
