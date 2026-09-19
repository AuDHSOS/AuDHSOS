# net-http and net-stack audit findings

Repository: AuDHSOS/AuDHSOS. Audit of net-http, net-stack at main bc47df5.
Each `## ` section is one issue, filed 2026-09-19 with the number in its heading. `Title:` is the issue title, `Labels:` the labels, everything after `Body:` is the issue body verbatim.

---

## F01 — issue #358
Title: net-stack: a peer whose every frame earns an answer stops every timer of the stack
Labels: bug, part::net
Body:
`Stack::poll` hands the received frame to `on_frame` first and asks the layers for timed work only when the outgoing queue is empty afterwards (`crates/net/stack/src/drive.rs:57-63`). An answer written by `on_frame` (an ARP reply, an ICMP echo reply, a TCP reset) makes the queue non-empty, so `drive` is skipped on that call; the answer is popped at `crates/net/stack/src/drive.rs:63`, and the next call with a frame repeats the pattern. `drive` runs DHCP renewal, DAD, the resolver, neighbor solicitation and TCP retransmission (`crates/net/stack/src/drive.rs:87-100`).

The serving thread takes one frame from the device per round and polls once per round for 64 rounds (`crates/user/net-programs/src/bin/server_net.rs:391-418`, `crates/user/net-programs/src/bin/server_net.rs:425`); with the receive ring never empty, every round carries a frame. A station on the link sending ARP requests for this host's address, or ICMP echo requests, faster than the server drains its ring keeps `drive` from running for as long as the flood lasts: TCP connections send no retransmission and no delayed ACK, the DHCP lease reaches T1, T2 and expiry without a renewal, and a pending DAD never completes.

Fix: decide whether to drive on the queue state before `on_frame`, or run `drive` whenever `poll_at(now)` is due regardless of the queue; the option not taken, a fixed budget of frames per drive, leaves the timers late by the size of the budget.

---

## F02 — issue #360
Title: net-stack: a datagram with a broadcast source address is answered to the whole link
Labels: bug, part::net
Body:
`on_ipv4` answers ICMP echo requests and refused TCP segments to `datagram.source()` without checking that the source is a unicast address (`crates/net/stack/src/receive.rs:175`, `crates/net/stack/src/receive.rs:219`). `transmit` sends any datagram whose destination is `255.255.255.255` through `broadcast` with no route lookup (`crates/net/stack/src/stack.rs:580-583`). `Datagram::parse` validates no address (`crates/net/ip/src/header.rs:112-135`). RFC 1122, section 3.2.1.3 (`docs/rfc/rfc1122.txt:1843-1846`) requires a host to discard a datagram whose source address is invalid, the limited broadcast among them; section 4.2.3.10 (`docs/rfc/rfc1122.txt:6117-6119`) says the same for a SYN.

One frame from the link carrying an echo request, or a SYN to a port with no listener, with source `255.255.255.255` and destination one of this host's addresses makes this host broadcast an echo reply or a reset to every station on the link. A multicast source reaches the same code and leaves through the default route.

Fix: drop a datagram in `on_ipv4` when `datagram.source()` is broadcast, multicast or unspecified, and the same for an IPv6 packet whose source is multicast in `on_ipv6`; the option not taken, refusing in `broadcast`, leaves the multicast case.

---

## F03 — issue #362
Title: net-stack: a SYN to a broadcast or multicast address is answered with a reset
Labels: bug, part::net
Body:
`accepts_v4` takes every broadcast and multicast destination (`crates/net/stack/src/receive.rs:223-233`) and `accepts_v6` every multicast destination (`crates/net/stack/src/receive.rs:301-303`). `on_ipv4` and `on_ipv6` hand a TCP segment with such a destination to `tcp_answer` (`crates/net/stack/src/receive.rs:195-202`, `crates/net/stack/src/receive.rs:273-280`), and `Connections::receive` builds a reset for any segment that matches no connection (`crates/net/tcp/src/table.rs:196-215`). RFC 9293, section 3.9.2.3, MUST-57 (`docs/rfc/rfc9293.txt:2877-2878`) requires a TCP to silently discard a SYN addressed to a broadcast or multicast address; RFC 1122, section 4.2.3.10 (`docs/rfc/rfc1122.txt:6121-6123`) says the same.

A SYN to `255.255.255.255` or to `ff02::1` from any station makes every host running this stack answer with a reset from its unicast address (`crates/net/stack/src/receive.rs:210-217`), one reply per host per frame.

Fix: in `on_ipv4` and `on_ipv6`, hand a TCP segment to `tcp_answer` only when `destination` is one of this host's addresses.

---

## F04 — issue #364
Title: net-stack: a datagram longer than 512 bytes to an unresolved neighbor is lost and no solicitation is sent
Labels: bug, part::net
Body:
The neighbor cache holds one pending packet of `HELD` = 512 bytes per neighbor (`crates/net/stack/src/stack.rs:43-44`). `NeighborCache::resolve` stores the packet before it inserts a new entry and answers `Dropped` without inserting when the packet does not fit (`crates/net/eth/src/neighbor.rs:230-275`, `crates/net/eth/src/neighbor.rs:507-515`), so no entry exists and `poll` never asks for that address. `hold` maps this to `Sent::Dropped` (`crates/net/ip/src/send.rs:183-196`). `transmit_v4` and `transmit_v6` act only on `Sent::Resolving` and return `Ok(())` for `Sent::Dropped` (`crates/net/stack/src/stack.rs:627-635`, `crates/net/stack/src/stack.rs:710-718`).

`send_to` with a 600-byte payload to an address whose next hop is not in the cache returns `Ok(())`, the datagram is not sent, and no ARP request or neighbor solicitation leaves; every retry has the same result until a packet under 512 bytes to the same next hop resolves it. A TCP connection whose next-hop entry was removed after failed probes (`crates/net/eth/src/neighbor.rs:415-470`) retransmits full segments of 1480 bytes into the same path and times out while the peer that sends only ACKs is never resolved again.

Fix: when the sender answers `Sent::Dropped` because the neighbor is unknown, start resolution with an empty held packet (a call of `resolve` with `&[]`) and report the loss to the caller of `send_to` with an error; the option not taken, raising `HELD` to the MTU, costs 16 KiB of cache and still drops a fragmented datagram.

---

## F05 — issue #366
Title: net-stack: a removed address stays the source of the sockets and connections bound to it
Labels: bug, part::net
Body:
`reconcile_lease` takes the address of an expired or replaced lease out of the address table and its routes out of the routing table (`crates/net/stack/src/drive.rs:187-197`); `drive_slaac` does the same for an expired prefix (`crates/net/stack/src/drive.rs:246-255`). `remove_address` removes the table entry and nothing else, although its doc says it forgets what was learned about the address (`crates/net/stack/src/stack.rs:271-278`). `drive_connections` sends from `connection.local().address` (`crates/net/stack/src/drive.rs:320`), `send_to` from `socket.local()` (`crates/net/stack/src/stack.rs:430-433`), and `transmit` checks no source (`crates/net/stack/src/stack.rs:572-592`). RFC 2131, section 4.4.5 (`docs/rfc/rfc2131.txt:2271-2274`) has a client stop all network processing with the expired address.

After the lease expires, or is renewed with a different address, every established connection keeps sending segments and retransmissions from the old address; `accepts_v4` drops the peer's answers (`crates/net/stack/src/receive.rs:223-233`), so each connection lives on until its retransmission limit, while segments with a source address this host no longer owns go onto the link. A UDP socket bound to the old address keeps sending from it without error.

Fix: in `remove_address`, abort every connection whose local endpoint carries the address and close every socket bound to it, returning their buffers through a state the caller can poll; the option not taken, refusing the send in `transmit`, leaves the connection state alive without a signal to the caller.

---

## F06 — issue #368
Title: net-stack: a candidate that cannot be connected to loses the caller's buffers
Labels: bug, part::net
Body:
`try_next_candidate` takes the two buffers out of the attempt and moves them into `connect_to` (`crates/net/stack/src/resolve.rs:337-351`). `connect_to` returns `NoAddress` before `Connections::connect` when `source_for` finds no address of the candidate's family (`crates/net/stack/src/stack.rs:489`), and the moved buffers are dropped with the error. `attempt.buffers` is `None` afterwards and `attempt.handle` is `None`; the next `try_next_candidate` finds no buffers and returns `Ok(())` (`crates/net/stack/src/resolve.rs:338-340`).

`connect_to_name` on a host with an IPv4 address only, for a name with an A and an AAAA record whose IPv4 host does not answer: the ordered list ends with the IPv6 address (`crates/net/stack/src/select.rs:314-318` sorts it last but keeps it), `retire_dead_candidate` closes the timed-out connection, `try_next_candidate` moves the buffers into `connect_to`, which fails with `NoAddress`; `poll` returns `Err(NoAddress)` once, `connecting()` answers `Failed` afterwards, and `abandon()` answers `None`. The caller never gets its send and receive buffers back. `connect_to_any` with one IPv6 candidate on the same host loses them on the first call.

Fix: skip a candidate whose `source_for` is `None` before taking the buffers, and treat an error from `connect_to` as that candidate's failure by putting the buffers back into the attempt, which requires `Connections::connect` to return them on error; the option not taken, dropping such candidates in `take_resolved`, leaves `connect_to_any` exposed.

---

## F07 — issue #369
Title: net-stack: a stale attempt handle makes every poll fail and stops all connections
Labels: bug, part::net
Body:
`retire_dead_candidate` resolves `attempt.handle` with `connection(handle)?` and propagates `Stale` and `Unknown` (`crates/net/stack/src/resolve.rs:300-307`). `drive` runs `drive_attempt` with `?` before `drive_connections` (`crates/net/stack/src/drive.rs:97-98`), so the error ends every later `poll` before any connection is polled. `connecting()` hands the handle out while the attempt still holds it (`crates/net/stack/src/resolve.rs:214-225`).

A caller that reads `Connecting::Open(handle)` and closes that handle with `close_connection` instead of `take_connection` leaves `attempt.handle` set to a retired generation; from then on `poll` returns `Err(Stale)` on every call, no connection sends anything, and the serving loop returns on the error at `crates/user/net-programs/src/bin/server_net.rs:407-409` every round until the caller calls `abandon`, which is the one call that clears the attempt (`crates/net/stack/src/resolve.rs:235-242`).

Fix: in `retire_dead_candidate`, treat `Stale` and `Unknown` as a dead candidate by clearing `attempt.handle` and continuing; the option not taken, hiding the handle from `connecting()` until `take_connection`, changes the public state enum.

---

## F08 — issue #370
Title: net-stack: an answer to a multicast or broadcast packet is checksummed over an address it does not leave from
Labels: bug, part::net
Body:
`on_ipv6` writes the ICMPv6 reply with `packet.destination()` as the checksum source (`crates/net/stack/src/receive.rs:267`, `crates/net/stack/src/receive.rs:432`, pseudo-header at `crates/net/ipv6/src/icmp.rs:297-302`) and then sends it from `source_for(source)` when the destination is not one of this host's addresses (`crates/net/stack/src/receive.rs:288-297`). `on_ipv4` writes a TCP reset the same way (`crates/net/stack/src/receive.rs:403`, `crates/net/stack/src/receive.rs:210-219`).

An echo request to `ff02::1` from a station on the link is answered with an echo reply whose IPv6 source is this host's unicast address and whose checksum was computed over `ff02::1`; the peer drops the reply as corrupt. RFC 4443, section 4.2 (`docs/rfc/rfc4443.txt:810-813`) requires the reply to a multicast echo to be sent from a unicast address of the interface, which the header does and the checksum does not.

Fix: choose `from` before writing the answer and pass it as the checksum source to `icmpv6_answer` and `tcp_answer`.

---

## F09 — issue #371
Title: net-stack: the lowest-numbered connection with data to send starves the others
Labels: bug, part::net
Body:
`drive_connections` walks the connection table from index 0, sends the first segment any connection offers, and stops (`crates/net/stack/src/drive.rs:309-327`). `drive` runs only when the outgoing queue is empty (`crates/net/stack/src/drive.rs:60-62`), so one segment leaves per drive, and the walk starts at index 0 every time.

Two connections, the one at index 0 sending a large body into an open peer window: `Connection::poll` on index 0 answers a segment on every drive while data and window remain, and index 1 sends no ACK, no retransmission and no FIN for that whole time; its peer retransmits into a connection that is alive and silent. The delay is bounded by the peer window of index 0, and repeats each time that window opens again.

Fix: keep the index the last walk stopped at in the stack and start the next walk one past it, so every connection with work sends within `CONNECTIONS` drives.

---

## F10 — issue #372
Title: net-stack: a bind or connect on a slot whose generation ran out leaks the socket
Labels: bug, part::net
Body:
`bind`, `bind_ephemeral`, `listen` and `connect_to` open the socket or connection in the lower table first and ask `Slots::handle` for the handle afterwards (`crates/net/stack/src/stack.rs:360-368`, `crates/net/stack/src/stack.rs:375-383`, `crates/net/stack/src/stack.rs:460-473`, `crates/net/stack/src/stack.rs:481-496`). `handle_of` answers `Full` for a slot whose generation is `u16::MAX` (`crates/net/stack/src/handle.rs:235-246`), and `retire_in` saturates the generation there (`crates/net/stack/src/handle.rs:262-266`). `Sockets::bind` fills the first free slot (`crates/net/udp/src/socket.rs:207-233`).

After 65534 open-and-close cycles on slot 0, the next `bind` on that slot returns `Err(Full)` with the socket already in the table: its port stays bound for the life of the stack, and the buffer the caller moved in is unreachable, since no handle names it and `close` needs one. `listen` and `connect_to` leave a connection and two buffers the same way.

Fix: ask `Slots::handle` for the slot the lower table will use before opening, or reserve a free slot whose generation is below `u16::MAX` and open into that one.

---

## F11 — issue #373
Title: net-stack: abandoning a connection by name leaves the resolver running
Labels: bug, part::net
Body:
`abandon` takes the attempt and returns its buffers (`crates/net/stack/src/resolve.rs:235-242`) but leaves `resolver` and `resolver_socket` set when the attempt was still resolving. `resolve` answers `Busy` while `resolver` is `Some` (`crates/net/stack/src/resolve.rs:55-57`), and `connect_to_name` calls `resolve` (`crates/net/stack/src/resolve.rs:200`).

`connect_to_name`, then `abandon` before the resolver finishes, then `connect_to_name` again: the second call returns `Err(Busy)`, and every later one does too, until the caller calls `forget_resolution` on its own. `drive_resolver` keeps reading the caller's socket for the abandoned query in the meantime (`crates/net/stack/src/resolve.rs:92-104`).

Fix: call `forget_resolution` in `abandon` when the attempt was resolving.

---

## F12 — issue #374
Title: net-stack: a router advertisement from a non-link-local source is accepted
Labels: bug, part::net
Body:
`on_discovery` hands a router advertisement to `slaac.on_advertisement` with `packet.source()` and no check of the source (`crates/net/stack/src/receive.rs:331-339`); `Configuration::on_advertisement` stores that source as the default router (`crates/net/ipv6/src/slaac.rs:297-321`), and `ndp::receive` checks the hop limit, the checksum and the code only (`crates/net/ipv6/src/ndp.rs:220-243`). RFC 4861, section 6.1.2 (`docs/rfc/rfc4861.txt:2174-2180`) requires a node to discard a router advertisement whose IP source address is not link-local.

An advertisement with a global source address and hop limit 255 from a station on the link installs that address as the default router; `transmit_v6` then resolves a global address as a neighbor for every off-link packet.

Fix: return from the `RouterAdvertisement` arm of `on_discovery` when `source.is_link_local()` is false.

---

## F13 — issue #375
Title: net-http: the feed doc tells the caller to stop on NeedMore while input remains
Labels: bug, part::net
Body:
The doc of `Decoder::feed` says a caller loops over what is left until the answer is `Event::NeedMore` or the message is done (`crates/net/http/src/response.rs:324-329`). `feed_head` consumes one complete line and answers `(end, Event::NeedMore)` when the head is not yet complete, whatever follows in `input` (`crates/net/http/src/response.rs:407-414`); the chunk-size and trailer arms answer `NeedMore` the same way after one line (`crates/net/http/src/response.rs:555-565`).

A caller written to the doc feeds a whole response `HTTP/1.1 200 OK\r\nContent-Length: 5\r\n\r\nhello`, receives `(17, NeedMore)` after the status line, and waits for bytes that never come. The crate's own test loop continues while `consumed > 0` (`crates/net/http/src/tests/response.rs:39-54`).

Fix: state the contract the test uses, a caller loops while the answer consumed at least one byte, in the doc of `feed`; the option not taken, consuming several lines per call, is what the module doc rules out at `crates/net/http/src/response.rs:6-10`.

---

## F14 — issue #376
Title: net-http: whitespace before a chunk extension is refused
Labels: bug, part::net
Body:
`chunk_size` cuts the size line at the first `;` and requires every byte before it to be a hex digit (`crates/net/http/src/response.rs:712-731`). RFC 9112, section 7.1.1 (`docs/rfc/rfc9112.txt:1093-1094`) defines `chunk-ext = *( BWS ";" BWS chunk-ext-name ...)`, so bad whitespace between the size and the `;` is part of the grammar a recipient parses.

A chunked body whose size line is `5 ;name=value\r\n` makes `feed` answer `Err(HttpError::ChunkSize)` and fail the response.

Fix: trim trailing spaces and tabs from `digits` before parsing them.

---

## F15 — issue #377
Title: net-http: a POST with no body is written without Content-Length
Labels: bug, part::net
Body:
`Request::write` writes `Content-Length` only when the body is non-empty (`crates/net/http/src/request.rs:333-337`), for every method. RFC 9110, section 8.6 (`docs/rfc/rfc9110.txt:3218-3222`) has a user agent send `Content-Length` in a request when the method defines a meaning for content, with `Content-Length: 0` for a POST without content named as the example.

A `Request { method: Method::Post, body: &[], .. }` goes out with no length field; an origin server that requires a length answers 411.

Fix: write `Content-Length` whenever `method` is `Post`, and keep it out for `Get` and `Head` when the body is empty.

---

## F16 — issue #378
Title: net-http: a request target may carry bytes outside origin-form
Labels: bug, part::net
Body:
`check_target` accepts every graphic ASCII byte behind a leading slash (`crates/net/http/src/request.rs:355-362`). RFC 9112, section 3.2.1 (`docs/rfc/rfc9112.txt:450-454`) defines `origin-form = absolute-path [ "?" query ]`, whose characters are `unreserved`, `pct-encoded`, `sub-delims`, `:`, `@`, `/` and `?`; `#`, `"`, `<`, `>`, `\`, `^`, `` ` ``, `{`, `|` and `}` are outside it, and RFC 9110, section 4.1 (`docs/rfc/rfc9110.txt:1030-1037`) excludes the fragment from protocol elements.

`Request::get("/page#top", host)` writes `GET /page#top HTTP/1.1`; a server answers 400 or reads a path this client did not mean.

Fix: refuse a byte outside the origin-form set in `check_target`, and refuse `#` in particular.

---

## F17 — issue #379
Title: net-http: a Host value may carry spaces and tabs
Labels: bug, part::net
Body:
`Request::write` checks the host with `is_value`, which allows space and tab inside the text (`crates/net/http/src/request.rs:310-312`, `crates/net/http/src/field.rs:181-186`). RFC 9112, section 3.2 defines `Host = uri-host [ ":" port ]`, and a `reg-name` holds no whitespace.

`Request::get("/", "example.com evil")` writes `Host: example.com evil`, which a server answers with 400.

Fix: check the host against the `uri-host` characters (`unreserved`, `pct-encoded`, `sub-delims`, `[`, `]`, `:`) instead of `is_value`.

---

## F18 — issue #380
Title: net-http: the TransferEncoding doc names an accepted case the decoder refuses
Labels: bug, part::net
Body:
The doc of `HttpError::TransferEncoding` says the error is for a `Transfer-Encoding` other than `chunked`, or one with `chunked` anywhere but last (`crates/net/http/src/error.rs:75-77`), which reads as `gzip, chunked` being accepted. `framing` accepts exactly one field whose trimmed value is `chunked` (`crates/net/http/src/response.rs:468-471`), and the test refuses `gzip, chunked` (`crates/net/http/src/tests/response.rs:275-287`).

A reader of the doc expects a response with `Transfer-Encoding: gzip, chunked` to decode and gets `Err(TransferEncoding)`.

Fix: state in the doc that the one accepted value is `chunked` alone, which D-50 (no content codings) explains.

---

## F19 — issue #381
Title: net-http: a header value with a non-UTF-8 byte is reported as a bad name
Labels: bug, part::net
Body:
`parse_field` converts the name and the value with `from_utf8` in one pattern and answers `HeaderName` when either fails (`crates/net/http/src/response.rs:644-649`); `is_value` is asked only afterwards (`crates/net/http/src/response.rs:656-659`).

A field line `X: \xff\r\n` decodes to `Err(HeaderName)`, and `X: \xc3\xa9\r\n`, a value with a non-ASCII byte that is valid UTF-8, to `Err(HeaderValue)`; the same class of value gets two errors.

Fix: check the value's bytes with `is_value_byte` before `from_utf8` and answer `HeaderValue` for a failing byte.

---

## F20 — issue #382
Title: net-stack: the scope_v4 doc says every non-link-local IPv4 address is global while the code assigns multicast scopes
Labels: bug, part::net
Body:
The doc of `scope_v4` says RFC 6724, section 3.1 gives the loopback and link-local ranges link-local scope and everything else global scope (`crates/net/stack/src/select.rs:153-155`). The body assigns `224.0.0.0/24` link-local scope and `239.0.0.0/8` site-local scope (`crates/net/stack/src/select.rs:161-166`). RFC 6724, section 3.2 (`docs/rfc/rfc6724.txt:444-453`) assigns `169.254/16` and `127/8` link-local scope and every other IPv4 address global scope.

`order_destinations` ranks `224.0.0.1` before a global unicast address under rule 8 (`crates/net/stack/src/select.rs:341-345`) for a caller that hands it a multicast candidate; a reader of the doc expects the opposite.

Fix: remove the two multicast arms from `scope_v4`, so that the code matches the doc and the document; the option not taken, keeping the arms and citing the reason in the doc, leaves the deviation.

---

## F21 — issue #383
Title: net-stack: drive_dhcp copies the datagram into a second MTU-sized buffer it does not need
Labels: enhancement, part::net
Body:
`drive_dhcp` holds two 1500-byte arrays and copies `outgoing.datagram` from the first into the second before calling `transmit` (`crates/net/stack/src/drive.rs:166-181`). `Client::poll` returns an `Outgoing<'b>` that borrows `buffer` and not the client (`crates/net/dhcp/src/client.rs:306-311`, `crates/net/dhcp/src/client.rs:193-201`), so `transmit(&mut self, ...)` can be called with `outgoing.datagram` directly.

Every drive puts 3000 bytes on the stack for this function, on the same path as `transmit_v4`'s 1518-byte frame (`crates/net/stack/src/stack.rs:614`), and every DHCP message is copied once more than it is written, O(n) in the datagram length.

Fix: drop `datagram` and pass `outgoing.datagram` to `transmit`.

---

## F22 — issue #384
Title: net-stack: try_next_candidate takes an instant it discards
Labels: enhancement, part::net
Body:
`try_next_candidate` takes `now` and discards it with `let _ = now;` (`crates/net/stack/src/resolve.rs:321-342`); `connect_to`, the one call it makes, takes no instant (`crates/net/stack/src/stack.rs:481-488`).

The parameter is dead code that reads as if the time were used to open the connection.

Fix: remove the parameter from `try_next_candidate` and pass `now` no further than `drive_attempt` needs it.

---

## F23 — issue #385
Title: net-stack: MESSAGE_LEN and ALL_NODES are each defined twice
Labels: enhancement, part::net
Body:
`MESSAGE_LEN` is a public constant of `stack` (`crates/net/stack/src/stack.rs:79-80`) and a private one of `drive` with the same value (`crates/net/stack/src/drive.rs:29-32`); `receive` imports the first (`crates/net/stack/src/receive.rs:26`). `ALL_NODES` is defined in `receive` (`crates/net/stack/src/receive.rs:28-31`) and again in `drive` (`crates/net/stack/src/drive.rs:34-37`), and `all_nodes` returns the second (`crates/net/stack/src/drive.rs:341-345`).

A change of one copy leaves the other, and the buffer-size reasoning in the comments at `crates/net/stack/src/drive.rs:29-32` and `crates/net/stack/src/receive.rs:94-96` refers to two constants that happen to agree.

Fix: keep the `stack` constant and the `drive` `ALL_NODES`, and import them in the other module.

---

## F24 — issue #386
Title: net-http: an overlong trailer line is reported as a bad chunk size
Labels: enhancement, part::net
Body:
`chunk_line` answers `HttpError::ChunkSize` when a line does not fit the 256-byte scratch, in every chunk state (`crates/net/http/src/response.rs:575-588`); the `Trailer` state reads trailer fields through it (`crates/net/http/src/response.rs:522-531`). The doc of `ChunkSize` names a size line that is too long and nothing else (`crates/net/http/src/error.rs:78-80`).

A chunked response whose trailer field line is 300 bytes long fails with `ChunkSize` although every chunk size was well formed.

Fix: answer `HttpError::Chunk` for an overlong line in the `After` and `Trailer` states, and state in the doc of `ChunkSize` which lines it covers.
