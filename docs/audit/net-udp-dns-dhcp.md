# net-udp, net-dns, net-dhcp audit findings

Repository: AuDHSOS/AuDHSOS. Audit of net-udp, net-dns, net-dhcp at main bc47df5.
Each `## ` section is one issue, filed 2026-09-19 with the number in its heading. `Title:` is the issue title, `Labels:` the labels, everything after `Body:` is the issue body verbatim.

---

## F01 — issue #271
Title: net-dhcp: a reply whose options continue in the file or sname field is ignored
Labels: bug, part::net
Body:
`Message::parse` skips the `sname` and `file` fields at `crates/net/dhcp/src/message.rs:185-189` and hands out only the bytes behind the magic cookie as `options` at `crates/net/dhcp/src/message.rs:204`. No code reads option 52 (option overload): `OptionCode` at `crates/net/dhcp/src/option.rs:26-53` has no constant for it, and `body_of` at `crates/net/dhcp/src/client.rs:621-630` walks `message.options()` alone. RFC 2131, section 4.1 (`docs/rfc/rfc2131.txt:1302-1308`) states that the `file` field MUST be interpreted next when the option overload option names it, and RFC 2132, section 9.3 (`docs/rfc/rfc2132.txt:1417-1426`) defines the option. The module doc at `crates/net/dhcp/src/message.rs:16-19` records the omission as a choice.

A server that sends option 52 with value 1 and places the message type (option 53) in `file` produces a reply for which `Message::message_type` at `crates/net/dhcp/src/message.rs:252-267` returns `MissingMessageType`, and `Client::on_datagram` at `crates/net/dhcp/src/client.rs:385-387` drops the reply. A server that places the subnet mask, the lease time, or the server identifier in `file` produces an acknowledgment for which `Lease::from_reply` at `crates/net/dhcp/src/client.rs:157-171` returns `MissingOption`, and `take_lease` at `crates/net/dhcp/src/client.rs:552-555` drops it. The client retransmits its request four times, restarts with a discover, and repeats without end against such a server.

Fix: read option 52 from the option field first, then run `Options::new` over the `file` field and then over the `sname` field when the value names them, with `body_of` and `message_type` searching all three blocks in that order; the option not taken, keeping the omission and stating in the README that a server which overloads is one this client does not bind with, leaves the MUST of section 4.1 unmet.

---

## F02 — issue #274
Title: net-dhcp: the request of the requesting state carries a later secs value than the discover
Labels: bug, part::net
Body:
`Client::plan` at `crates/net/dhcp/src/client.rs:443-447` computes `secs` from `now.saturating_duration_since(self.started)` on every poll, and `started` is set only by `begin` at `crates/net/dhcp/src/client.rs:407` and on the renewal at `crates/net/dhcp/src/client.rs:430`. RFC 2131, section 3.1, step 3 (`docs/rfc/rfc2131.txt:858-862`) states that the DHCPREQUEST MUST use the same value in the `secs` field as the original DHCPDISCOVER, so that relay agents forward it to the same set of servers.

A discover sent at `t = 0` carries `secs = 0`; an offer arrives at `t = 5 s`; the request written by the next poll carries `secs = 5`. A relay agent that forwards to a second server only when `secs` reaches a threshold forwards the request to servers that did not see the discover. The test at `crates/net/dhcp/src/tests/client.rs:124-181` sends every message at one instant and does not observe the difference.

Fix: store the `secs` value written into the last discover in `Client` and write that value into every request of the requesting state, computing a fresh value only for a discover and for the renewing and rebinding states; the option not taken, resetting `started` in `take_offer`, produces `secs = 0` in the request, which is not the discover's value either.

---

## F03 — issue #277
Title: net-dhcp: an acknowledgment from a server the request did not name is taken as the lease
Labels: enhancement, part::net
Body:
`Client::on_datagram` at `crates/net/dhcp/src/client.rs:390-392` passes every acknowledgment with a matching `xid` and `chaddr` to `take_lease`, and `take_lease` at `crates/net/dhcp/src/client.rs:552-561` builds the lease from the acknowledgment without comparing its server identifier with `self.offer.server` or its `yiaddr` with `self.offer.address`. The request names the selected server in the server identifier option at `crates/net/dhcp/src/client.rs:588-592`, and RFC 2131, section 3.1, step 4 (`docs/rfc/rfc2131.txt:866-872`) has only the selected server answer with an acknowledgment.

Two servers A and B offer addresses; the client requests A's offer with A as the server identifier; B answers with an acknowledgment carrying its own server identifier and its own `yiaddr`. The client binds B's address while A commits its binding for this client. The client holds an address A never granted and A holds one the client never uses.

Fix: in the requesting state, accept an acknowledgment only when its server identifier equals `offer.server` and its `yiaddr` equals `offer.address`, dropping the rest as another server's reply; the option not taken, comparing the server identifier alone, still admits an acknowledgment from A for a different address.

---

## F04 — issue #280
Title: net-dns: the response check compares the question name and type and not the class
Labels: bug, part::net
Body:
`Resolver::on_datagram` at `crates/net/dns/src/resolver.rs:455-461` matches a response to a question by `id`, `record_type`, and `name` and reads `question.class` nowhere. `Question::read` at `crates/net/dns/src/message.rs:325-333` decodes the class. The README at `crates/net/dns/README.md:36-38` and the module doc at `crates/net/dns/src/resolver.rs:13-16` state that the question section must be the question that was asked; the query wrote class `IN` at `crates/net/dns/src/message.rs:300-306`.

A response with the right id, name, and type and class `3` (CH) in its question passes the check. Its `IN` answer records are then walked at `crates/net/dns/src/resolver.rs:513-536` and settle the question, so a response that does not echo the question the resolver asked is believed.

Fix: add `question.class == Class::IN` to the predicate at `crates/net/dns/src/resolver.rs:455-461`; the option not taken, storing the asked class in `Ask`, adds a field for a value that is always `IN`.

---

## F05 — issue #283
Title: net-dns: the name reader's docs claim the backward rule alone makes the walk finite
Labels: bug, part::net
Body:
The module doc at `crates/net/dns/src/name.rs:14-22` states that a pointer which points strictly backwards makes the walk "provably move towards the front of the message" so that it "can never return to where it has been", and the constant doc at `crates/net/dns/src/name.rs:42-50` states that `MAX_JUMPS` "is not what makes it finite". The README at `crates/net/dns/README.md:9-15` repeats the claim. `Name::read` at `crates/net/dns/src/name.rs:132-163` advances `cursor` forward over every label it reads (`crates/net/dns/src/name.rs:145`) and compares a pointer against that advanced cursor (`crates/net/dns/src/name.rs:151`), so a pointer behind a label may point at bytes the walk already read.

The message `01 61 C0 00` read from offset 0 yields label `a`, moves the cursor to 2, reads the pointer to 0, which is backwards, and returns to offset 0. The walk cycles and ends only by `NameTooLong` at `crates/net/dns/src/name.rs:138` or `PointerChain` at `crates/net/dns/src/name.rs:155-157`. The test at `crates/net/dns/src/tests/name.rs:242-257` builds exactly such a walk, which returns to bytes 0 to 64 five times, and ends by the 255-byte bound. The bounds that end the walk are `MAX_JUMPS` and `MAX_NAME_LEN`, which the docs name as secondary.

Fix: rewrite `crates/net/dns/src/name.rs:14-22`, `crates/net/dns/src/name.rs:42-50`, and `crates/net/dns/README.md:9-15` to state that the backward rule refuses a pointer to itself or forwards, and that the jump count and the 255-byte bound are what end a cycle through a label; the option not taken, comparing a pointer against the lowest offset the walk has visited, changes behavior for names a correct encoder produces (RFC 1035, section 4.1.4, `docs/rfc/rfc1035.txt:1638-1640`, allows a pointer to any prior occurrence).

---

## F06 — issue #286
Title: net-udp: the socket module doc describes a wildcard fallback the port rule excludes
Labels: bug, part::net
Body:
The module doc at `crates/net/udp/src/socket.rs:11-13` states that a datagram to a port whose socket is bound to another address "is offered to the wildcard socket instead". `Sockets::bind` at `crates/net/udp/src/socket.rs:216-218` refuses a second socket on a port that is held, whatever the address, so a port has at most one socket, and `lookup` at `crates/net/udp/src/socket.rs:294-303` finds no second candidate.

A socket bound to `Some(A)` on port 53 and a datagram to `B:53` produce `Delivery::PortUnreachable` at `crates/net/udp/src/socket.rs:275-277`. The test at `crates/net/udp/src/tests/socket.rs:145-159` asserts this. A reader of the module doc expects delivery to a wildcard socket that cannot exist.

Fix: rewrite `crates/net/udp/src/socket.rs:11-13` to state that a datagram to another address of a bound socket's port is reported as port unreachable.

---

## F07 — issue #289
Title: net-dns: the doc of Status::Done says both questions are settled where one failure suffices
Labels: bug, part::net
Body:
The variant doc at `crates/net/dns/src/resolver.rs:111-114` states `Done` means "Both questions are settled". `Resolver::status` at `crates/net/dns/src/resolver.rs:275-284` returns `Done` when no question is `Asking` and at least one is `Settled`, so one `Settled` and one `Failed` question is `Done`.

The `A` question answered with `NAME_ERROR` is settled at `crates/net/dns/src/resolver.rs:482-486`; the `AAAA` question runs out of attempts and is failed at `crates/net/dns/src/resolver.rs:382-384`; `status` returns `Done`, and the failure of the `AAAA` question is not visible through `Status`.

Fix: rewrite `crates/net/dns/src/resolver.rs:111-114` to state that `Done` means no question is open and at least one was answered, and that the failure of the other is dropped; the option not taken, a `Done` variant that carries the failure of the other question, changes the public type.
