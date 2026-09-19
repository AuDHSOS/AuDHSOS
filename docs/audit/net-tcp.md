# net-tcp audit findings

Repository: AuDHSOS/AuDHSOS. Audit of net-tcp at main bc47df5.
Each `## ` section is one issue, filed 2026-09-19 with the number in its heading. `Title:` is the issue title, `Labels:` the labels, everything after `Body:` is the issue body verbatim.

---

## F01 — issue #442
Title: net-tcp: the backed-off timeout wraps to zero after 58 probes
Labels: bug, part::net
Body:
`Rto::get` shifts the base timeout left by `attempts` with `checked_shl` at `crates/net/tcp/src/rto.rs:80-87`. `checked_shl` fails only for a shift of 64 or more; a shift of 58 to 63 moves every bit of a one-second base (1 000 000 = 15625 · 2^6) out of the `u64` and yields 0. `Rto::get` then answers a timeout of zero, below the floor `MINIMUM` at `crates/net/tcp/src/rto.rs:37` and against the claim at `crates/net/tcp/src/rto.rs:14-16`. `attempts` reaches 58 on the persist path: every window probe calls `rto.back_off` at `crates/net/tcp/src/connection.rs:1172` with no limit, and only a round-trip sample resets the counter, which no probe supplies.

A peer that advertises a zero window for 53 minutes (probes at 2, 4, 8, 16, 32 seconds, then 60 seconds each) drives `attempts` to 58. The next six probes leave in one `poll` loop, because `persist_at` at `crates/net/tcp/src/connection.rs:1173` is `now`. When the peer opens its window while `attempts` is between 58 and 63, `arm_retransmit` at `crates/net/tcp/src/connection.rs:1294-1300` sets `retransmit_at` to `now`, and each of the following six polls counts a retransmission at `crates/net/tcp/src/connection.rs:888-903`, retransmits the whole window, and consumes six of the eight attempts of `Config::DEFAULT`. With `retransmit_limit` below 6 the connection is torn down in that loop.

Fix: bound the shift in `Rto::get` to the smallest count that already reaches `MAXIMUM` (6 for a one-second base, computed from `MAXIMUM / base`) before shifting, so the result saturates at the ceiling; bounding `attempts` in `back_off` instead would lose the count that tests and `timed_out` read.

---

## F02 — issue #444
Title: net-tcp: a bare acknowledgment after a window probe carries a sequence number the peer rejects
Labels: bug, part::net
Body:
A window probe moves `snd_max` one past the probed byte at `crates/net/tcp/src/connection.rs:1168-1171` and leaves `snd_nxt`. `Plan::Ack` writes `snd_max` into the sequence field at `crates/net/tcp/src/connection.rs:1124-1133`. After one probe every acknowledgment that carries no data names `SND.UNA + 1`, while the peer, whose window is zero, has not taken the byte and expects `SND.UNA`.

RFC 9293, section 3.10.7.4 (`docs/rfc/rfc9293.txt:3499-3521`) accepts a segment of length zero at a zero window only when `SEG.SEQ` equals `RCV.NXT`; the peer answers with its own acknowledgment and drops ours. Trigger: the peer's receive window is zero, this end has written bytes and sent one probe, and the peer sends data into this end's open window. Every acknowledgment of that data leaves with sequence `SND.UNA + 1` and is dropped by the peer; the data is acknowledged only by the next probe, up to 60 seconds later, and the peer retransmits meanwhile. With this crate at both ends the peer also drops the probe (F03), retransmits its data until `retransmit_limit` is spent, and tears the connection down.

Fix: send the probe as a segment of length zero at `SND.UNA - 1`, which RFC 1122, section 4.2.2.17 (`docs/rfc/rfc1122.txt:5376-5410`) allows and which draws the peer's acknowledgment with its window because it is unacceptable, so that `snd_max` stays where the peer's `RCV.NXT` is; the option not taken, writing `snd_nxt` into a bare acknowledgment while `snd_wnd` is zero, is wrong after a retransmission timeout rewound `snd_nxt`.

---

## F03 — issue #447
Title: net-tcp: a zero receive window drops the acknowledgment a data segment carries
Labels: bug, part::net
Body:
`is_acceptable` answers `false` for every segment with a length above zero while the window is zero at `crates/net/tcp/src/seq.rs:128`. `on_segment_open` then sets `ack_now` and returns at `crates/net/tcp/src/connection.rs:603-614`, so `check_ack`, `update_window` and the round-trip measurement never see the segment. RFC 9293, section 3.10.7.4 (`docs/rfc/rfc9293.txt:3519-3521`) asks for a special allowance that accepts valid acknowledgments and resets in that case.

Trigger: this end's receive buffer is full, this end has data in flight, and the peer's acknowledgment of that data rides on a one-byte window probe or on a data segment. The acknowledgment is dropped, the retransmission timer expires, the data is sent again, and after `retransmit_limit` expiries `give_up` at `crates/net/tcp/src/connection.rs:1282-1290` tears the connection down although the peer acknowledged everything. A peer running this crate produces the case through F02: its bare acknowledgments name `SND.UNA + 1` and fail the `(0, 0)` test at `crates/net/tcp/src/seq.rs:126`.

Fix: when the window is zero and the segment fails the test, still run the RST check and `check_ack` (without `accept_text` and `check_fin`) for a segment whose `SEG.ACK` lies between `SND.UNA` and `SND.MAX`; the option not taken, accepting the segment as a whole, would write bytes past the window.

---

## F04 — issue #449
Title: net-tcp: a reset in SYN-RECEIVED closes a passive-open connection instead of returning it to LISTEN
Labels: bug, part::net
Body:
A reset at `RCV.NXT` in `SynReceived` calls `tear_down(true)` at `crates/net/tcp/src/connection.rs:615-626`, which sets `State::Closed` at `crates/net/tcp/src/connection.rs:1255-1269`. RFC 9293, section 3.10.7.4 (`docs/rfc/rfc9293.txt:3582-3590`) returns a connection that came from `LISTEN` to `LISTEN` on a reset and closes only one that came from `SYN-SENT`. The connection keeps no record of which open it came from.

Trigger: a peer sends `SYN`, receives the `SYN,ACK`, and answers with `RST` at the acknowledged number, which is what a half-open port scan does. The listener becomes `Closed` with `was_reset` set; `lookup` at `crates/net/tcp/src/table.rs:262-281` finds no listener for the next `SYN`, and `receive` answers it with a reset at `crates/net/tcp/src/table.rs:209-213` until the layer above opens a new listener (`crates/user/servers/net/src/server.rs:336-360`). One packet takes the service off the port.

Fix: record a passive origin in `on_segment_listen` at `crates/net/tcp/src/connection.rs:532-552`, and on a reset in `SynReceived` restore `State::Listen`, the unspecified remote endpoint and the initial numbers; draw the next initial sequence number from a secret kept since `listen` as RFC 6528, section 3 does (see F17), because `on_segment` has no generator; the option not taken, re-listening from the layer above, leaves the port answering with resets between the two.

---

## F05 — issue #452
Title: net-tcp: one unanswered SYN occupies the listener for three minutes and then closes it
Labels: bug, part::net
Body:
A `SYN` turns the listening connection into `SynReceived` at `crates/net/tcp/src/connection.rs:546-551`. Its `SYN,ACK` is retransmitted by `decide_syn` at `crates/net/tcp/src/connection.rs:937-945`, and `give_up` at `crates/net/tcp/src/connection.rs:1282-1290` closes the connection after `retransmit_limit` expiries. The listener is the connection (`crates/net/tcp/README.md:16-20`), so no other `SYN` is served meanwhile and none afterwards.

Trigger: one `SYN` with a spoofed source address. The `SYN,ACK` goes unanswered; with `Config::DEFAULT` the timer runs 1, 2, 4, 8, 16, 32, 60 and 60 seconds (183 seconds in all, `crates/net/tcp/src/rto.rs:33-40`), during which `lookup` at `crates/net/tcp/src/table.rs:273-277` finds no listener and every further `SYN` is refused with a reset. The connection then closes with `timed_out` set and the port stays refused until the layer above listens again.

Fix: on `give_up` in `SynReceived` of a passive-open connection restore `State::Listen` as in F04, so the listener is reusable after the embryonic connection dies; the option not taken, a backlog, is what the README rules out.

---

## F06 — issue #454
Title: net-tcp: every short segment postpones the delayed acknowledgment again
Labels: bug, part::net
Body:
`accept_text` sets `ack_at` to `now + delayed_ack` for every in-order segment shorter than `config.max_segment` at `crates/net/tcp/src/connection.rs:818-825`, and again for the first full segment at `crates/net/tcp/src/connection.rs:826-831`. An earlier deadline is overwritten each time; `ack_at` is cleared only by `sent` at `crates/net/tcp/src/connection.rs:1157-1158`. RFC 1122, section 4.2.3.2 (`docs/rfc/rfc1122.txt:5653-5656`) bounds the delay of an acknowledgment to below 0.5 seconds.

Trigger: the peer sends segments shorter than this end's announced segment size at intervals below 500 milliseconds, which any peer with a smaller effective MSS or an interactive writer does. No acknowledgment leaves until the peer's congestion window is exhausted and 500 milliseconds have passed since its last segment; the first byte of that burst waits for longer than the bound. The peer's window grows by one segment per 500 milliseconds.

Fix: keep the earlier deadline with `ack_at.get_or_insert(now + delayed_ack)` in both branches.

---

## F07 — issue #457
Title: net-tcp: a segment counts as full only against this end's own announced segment size
Labels: bug, part::net
Body:
`accept_text` compares the payload length with `config.max_segment` at `crates/net/tcp/src/connection.rs:818`, which is the size this end announces (`crates/net/tcp/src/connection.rs:70-72`). A peer whose own effective segment size is smaller sends no segment that reaches it, and the every-second-segment rule at `crates/net/tcp/src/connection.rs:826-831` never fires.

Trigger: this end announces 1460 and the peer's path clamps its MSS to 1452 or 1440, which routers on PPPoE and tunnel paths do. Every segment of the peer's stream is `short`, each takes the delayed branch, and with F06 fixed one acknowledgment leaves per 500 milliseconds; the peer sends one congestion window per 500 milliseconds and grows it by one segment per acknowledgment.

Fix: count every in-order data segment toward the second-segment rule and acknowledge at once on the second, as BSD does, instead of testing the size; the option not taken, estimating the peer's segment size from the segments that arrive, adds state for the same effect.

---

## F08 — issue #459
Title: net-tcp: a segment that begins before RCV.NXT rewrites acknowledged bytes
Labels: bug, part::net
Body:
`accept_text` computes the offset from the front of the receive buffer, which is `RCV.NXT` minus the unread bytes, at `crates/net/tcp/src/connection.rs:796-805`, and skips only the part of the payload that precedes that front. `RecvBuffer::accept` writes the whole slice with `put` at `crates/net/tcp/src/recv.rs:173` before it looks at `ready` at `crates/net/tcp/src/recv.rs:174-181`. Bytes between the front and `RCV.NXT`, which are received, acknowledged and unread, are overwritten. The doc comment at `crates/net/tcp/src/recv.rs:160-163` says bytes the run has already passed are ignored, and RFC 9293, section 3.10.7.4 (`docs/rfc/rfc9293.txt:3540-3546`) trims a segment to what lies beyond `RCV.NXT`.

Trigger: a segment with `SEG.SEQ` below `RCV.NXT` and above the front, whose end lies in the window, with a payload that differs from what was received the first time. The caller has `peek`ed the bytes at `crates/net/tcp/src/connection.rs:436-438` and not yet `consume`d them; they change between the two calls. A peer can do it with a retransmission that differs, and so can anyone who can place a segment in the window.

Fix: in `RecvBuffer::accept` start the write at `offset.max(self.ready)` and skip the bytes before it.

---

## F09 — issue #460
Title: net-tcp: an acknowledgment exactly half a circle ahead passes the acknowledgment check
Labels: bug, part::net
Body:
`check_ack` rejects an acknowledgment with `ack.after(self.snd_max)` at `crates/net/tcp/src/connection.rs:683-692` and takes it as new with `ack.after(self.snd_una)` at `crates/net/tcp/src/connection.rs:693`. When nothing is in flight, `snd_una` equals `snd_max`, and an acknowledgment number `SND.UNA + 2^31` satisfies neither `after` nor the equality at `crates/net/tcp/src/connection.rs:695`, which `crates/net/tcp/src/seq.rs:15-20` names as the case a test of two comparisons has to decide. `check_ack` returns `true`, and `update_window` at `crates/net/tcp/src/connection.rs:777-789` installs the segment's window and sets `snd_wl2` to that number.

Trigger: nothing in flight; a segment with `SEG.SEQ` in the window, `SEG.ACK = SND.UNA + 2^31` and a window of zero. `snd_wnd` becomes zero and this end starts probing. Every later acknowledgment of the peer at the same `SEG.SEQ` carries a real number, for which `snd_wl2.before_or_equal(ack)` at `crates/net/tcp/src/connection.rs:779` is false, so the peer's window is not taken until the peer sends data with a higher sequence number. A peer that only reads leaves the connection probing.

Fix: reject with `!ack.before_or_equal(self.snd_max)` in place of `ack.after(self.snd_max)`, which classifies the number exactly half a circle ahead as beyond `SND.MAX`.

---

## F10 — issue #461
Title: net-tcp: a peer's segment size below 536 is raised to 536
Labels: bug, part::net
Body:
`accept_peer` clamps the announced maximum segment size to at least `MIN_MAX_SEGMENT` (536) at `crates/net/tcp/src/connection.rs:1232-1237`, and `decide_data` sends segments up to that value at `crates/net/tcp/src/connection.rs:972-999`. RFC 9293, section 3.7.1 (`docs/rfc/rfc9293.txt:1745-1757`, MUST-16) bounds the effective send MSS by the value the peer announced. The comment at `crates/net/tcp/src/connection.rs:41-44` reads the 576-byte reassembly minimum as a floor on what a peer may ask for; the RFC states it as a floor on what a host must accept.

Trigger: the peer announces 500. This end sends 536-byte segments; the peer's IP layer fragments or drops them, depending on its path.

Fix: honor the announced value down to a floor of 88 bytes, which is the floor Linux applies against a peer that asks for one byte per segment; the option not taken, no floor, lets a peer cost this end one segment per byte.

---

## F11 — issue #462
Title: net-tcp: an active open takes the local port 0 and nothing draws an ephemeral port
Labels: bug, part::net
Body:
`Connections::connect` at `crates/net/tcp/src/table.rs:158-173` and `Connection::connect` at `crates/net/tcp/src/connection.rs:395-410` accept any local port, including `Port::UNSPECIFIED`, and this crate has no allocation of an ephemeral port. The caller passes `Port::new(0)` at `crates/net/stack/src/stack.rs:488-490`, so every active connection of the system has source port 0. `net-udp` draws its ephemeral ports at `crates/net/udp/src/socket.rs:242-250`.

Trigger: two `connect_to` calls to the same remote endpoint; the second fails with `PortInUse` at `crates/net/tcp/src/table.rs:167-169` because both name the four-tuple with local port 0. Any peer or firewall that refuses port 0 as a source (RFC 6335 reserves it) drops the `SYN`.

Fix: draw a port from the dynamic range with `rng` in `Connections::connect` when `local.port` is `UNSPECIFIED`, skipping every port a connection in any open state, `TimeWait` included, holds toward the same remote endpoint; the option not taken, allocation in `net-stack`, would leave the table accepting port 0 from other callers.

---

## F12 — issue #463
Title: net-tcp: a simultaneous open leaves SND.WL2 unset and drops the window of the SYN-ACK half the time
Labels: bug, part::net
Body:
`on_segment_syn_sent` sets `snd_wl1` and `snd_wl2` only when the arriving `SYN` carries an acknowledgment at `crates/net/tcp/src/connection.rs:574-579`; `accept_peer` sets `snd_wl1` to `SEG.SEQ` at `crates/net/tcp/src/connection.rs:1231` and leaves `snd_wl2` at its initial 0. The `SYN,ACK` that completes the open then reaches `update_window` from `on_repeated_syn` at `crates/net/tcp/src/connection.rs:666` with `SEG.SEQ` equal to `snd_wl1`, so the test at `crates/net/tcp/src/connection.rs:779` is `SeqNumber(0).before_or_equal(ISS + 1)`, which holds only for an `ISS` below `2^31 - 1`.

Trigger: both ends open at once, the peer's `SYN` advertises a window of 0 or a small one and its `SYN,ACK` a larger one, and this end's `ISS` is at or above `2^31 - 1`. `snd_wnd` keeps the `SYN`'s window until the peer sends a segment with a higher sequence number; with a zero window this end probes a peer that has room.

Fix: in `on_repeated_syn`, when the segment completes the open, set `snd_wnd`, `snd_wl1` and `snd_wl2` from the segment without the newer-segment test, as `on_segment_syn_sent` does for the `SYN,ACK` at `crates/net/tcp/src/connection.rs:574-579`.

---

## F13 — issue #464
Title: net-tcp: a retransmitted FIN in TIME-WAIT does not restart the 2 MSL timer
Labels: bug, part::net
Body:
The peer's retransmitted `FIN` occupies numbers below `RCV.NXT`, fails `is_acceptable` at `crates/net/tcp/src/seq.rs:129-132`, and leaves `on_segment_open` with `ack_now` at `crates/net/tcp/src/connection.rs:606-614`. The restart at `crates/net/tcp/src/connection.rs:858` is behind `check_fin`, which only a `FIN` at `RCV.NXT` reaches (`crates/net/tcp/src/connection.rs:840-844`). RFC 9293, section 3.10.7.4 (`docs/rfc/rfc9293.txt:3802-3805`) acknowledges the retransmission and restarts the 2 MSL timeout.

Trigger: this end enters `TimeWait` at `t`; the peer's `FIN` retransmission arrives at `t + 50 s`. The acknowledgment leaves; `time_wait_until` stays `t + 60 s` (`crates/net/tcp/src/connection.rs:1351-1353`), and the connection is closed 10 seconds after the peer last sent into it rather than 60. The line at `crates/net/tcp/src/connection.rs:858` restarts the timer only for a second `FIN` one past the first, which a conforming peer never sends.

Fix: in the unacceptable branch of `on_segment_open`, when the state is `TimeWait` and the segment carries `FIN`, set `time_wait_until` to `time_wait_end(now)`.

---

## F14 — issue #465
Title: net-tcp: the timeout is not re-initialized to three seconds after a lost SYN
Labels: bug, part::net
Body:
RFC 6298, section 5.7 (not under `docs/rfc/`) requires an RTO below 3 seconds to be re-initialized to 3 seconds when data transmission begins after the timer expired for a `SYN`. `decide_syn` backs the timer off at `crates/net/tcp/src/connection.rs:941-943` and clears the sample for Karn's rule at `crates/net/tcp/src/connection.rs:942`; the transitions to `Established` at `crates/net/tcp/src/connection.rs:580-585` and `crates/net/tcp/src/connection.rs:663-669` keep the estimate as it is.

Trigger: the first `SYN` is lost. `attempts` is 1 and `Rto::get` at `crates/net/tcp/src/rto.rs:80-87` answers 2 seconds; the first data segments run with a 2-second timeout and no sample, because the retransmitted `SYN` yields none. A path whose round trip is above 2 seconds retransmits its first data needlessly.

Fix: in both transitions to `Established`, when `rto.attempts()` is above 0 and `rto.get()` is below 3 seconds, reset the estimate and set its base to 3 seconds.

---

## F15 — issue #466
Title: net-tcp: the doc of Connections::close claims a reset that the code does not send
Labels: bug, part::net
Body:
The doc at `crates/net/tcp/src/table.rs:175-178` states that a connection still open sends a reset first, which the caller fetches with one last `poll` before it takes the buffers. `Connections::close` at `crates/net/tcp/src/table.rs:183-191` takes the slot and returns the buffers at once, without `abort` at `crates/net/tcp/src/connection.rs:466-472` and without a `poll`. The only caller confirms it: `crates/user/servers/net/src/server.rs:575-583` closes without a reset.

Trigger: an `Established` connection is closed through `Connections::close`. The peer learns nothing until its next segment, which `receive` answers with a reset at `crates/net/tcp/src/table.rs:209-213`; a peer that waits for this end sends nothing and holds its connection for its own keepalive or user timeout.

Fix: make `Connections::close` refuse a connection that is open and not `Listen` unless `abort` was called and polled, or state in the doc that no reset leaves and the peer's next segment draws one from the table; one of the two, because the doc and the code disagree.

---

## F16 — issue #467
Title: net-tcp: the doc of Rto::attempts names a give-up rule the connection does not use
Labels: bug, part::net
Body:
The doc at `crates/net/tcp/src/rto.rs:101-102` states that a connection gives up when `attempts` reaches the configured count. The connection counts its own `retries` at `crates/net/tcp/src/connection.rs:222-226` and compares that field with `retransmit_limit` in `give_up` at `crates/net/tcp/src/connection.rs:1282-1290`; `Rto::attempts` is read only by tests. `attempts` also grows on every window probe at `crates/net/tcp/src/connection.rs:1172`, where no give-up applies.

Trigger: none in behavior; a reader of `rto.rs` looks for a limit that `Config::retransmit_limit` applies elsewhere.

Fix: state in the doc that `attempts` counts doublings of the timeout since the last sample, on the retransmission path and on the persist path, and that `Connection::retries` is what the limit applies to.

---

## F17 — issue #468
Title: net-tcp: the connect doc claims RFC 6528 for a plain random initial sequence number
Labels: bug, part::net
Body:
The doc at `crates/net/tcp/src/connection.rs:388-389` states that the initial sequence number is drawn as RFC 6528 asks. RFC 6528, section 3 (not under `docs/rfc/`) generates `ISN = M + F(localip, localport, remoteip, remoteport, secretkey)` with a 4-microsecond clock `M`. `set_initial_sequence` at `crates/net/tcp/src/connection.rs:1242-1251` takes four random bytes and no clock, and `listen` draws the number before the peer is known at `crates/net/tcp/src/connection.rs:379-386`. D-51 (`docs/09-decisions.md:61`) decides the random draw; it does not name RFC 6528.

Trigger: none in behavior against a blind attacker, because the number is unpredictable; the monotonic property of RFC 6528, that a new incarnation of a four-tuple starts above the old one, is absent, and `abort` at `crates/net/tcp/src/connection.rs:466-472` closes without `TimeWait`, so a reincarnation after an abort may start inside the old window.

Fix: cite D-51 in the doc and describe the number as a random 32-bit value, or generate it as RFC 6528 section 3 does from a secret and the four-tuple plus a clock, which F04 also needs.

---

## F18 — issue #469
Title: net-tcp: the first window probe leaves at once instead of after one timeout
Labels: enhancement, part::net
Body:
`decide_probe` answers a probe when `persist_at` is `None` at `crates/net/tcp/src/connection.rs:963-966`, and `update_window` clears `persist_at` only when the window opens at `crates/net/tcp/src/connection.rs:786-788`. The first probe therefore leaves in the `poll` that follows the window closing; the test at `crates/net/tcp/src/tests/connection.rs:491-494` asserts that. RFC 9293, section 3.8.6.1 (`docs/rfc/rfc9293.txt:2189-2192`, SHLD-29) sends the first probe after a zero window has existed for one retransmission timeout.

Trigger: the peer's window closes and reopens within one round trip, which every peer does that acknowledges and reads at once. This end sends one probe per closure that a delayed first probe would not send, and each probe backs `rto` off at `crates/net/tcp/src/connection.rs:1172` (F01).

Fix: when `update_window` sets `snd_wnd` to zero with bytes waiting, set `persist_at` to `now + rto.get()` instead of leaving it `None`.

---

## F19 — issue #470
Title: net-tcp: CLOSE in SYN-RECEIVED never sends the FIN the memo allows
Labels: enhancement, part::net
Body:
`close` sets `closing` in `SynReceived` at `crates/net/tcp/src/connection.rs:458-461`; `fin_due` at `crates/net/tcp/src/connection.rs:1036` and the `fin` flag at `crates/net/tcp/src/connection.rs:1018` require `can_send`, which excludes `SynReceived` at `crates/net/tcp/src/state.rs:81-83`. RFC 9293, section 3.10.4 (`docs/rfc/rfc9293.txt:3142-3147`) sends a `FIN` and enters `FIN-WAIT-1` when nothing is queued, and section 3.3.2 draws that transition; the code always queues.

Trigger: a passive-open connection is closed by the caller before the peer's acknowledgment of the `SYN,ACK` arrives. The `FIN` waits for that acknowledgment or for a `SYN,ACK` retransmission to draw one; nothing is lost, one round trip is.

Fix: in `fin_due` and `decide_data` allow the `FIN` in `SynReceived` when the send buffer is empty, and in `sent` map `SynReceived` to `FinWait1`.

---

## F20 — issue #471
Title: net-tcp: abort in SYN-SENT sends a reset with an acknowledgment field of zero
Labels: enhancement, part::net
Body:
`abort` at `crates/net/tcp/src/connection.rs:466-472` plans a reset with `ack = Some(self.rcv_nxt)` for every open state but `Listen`. In `SynSent` no segment of the peer has arrived, `rcv_nxt` is the 0 of `Connection::new` at `crates/net/tcp/src/connection.rs:266`, and the reset leaves with `RST,ACK` and acknowledgment 0. RFC 9293, section 3.10.5 (`docs/rfc/rfc9293.txt:3192-3195`) deletes the TCB in `SYN-SENT` and sends nothing.

Trigger: `abort` on a connection whose `SYN` is unanswered. One segment leaves that acknowledges a number the peer never used; a peer in `SYN-RECEIVED` resets on its sequence number alone, so nothing further happens.

Fix: in `abort`, plan no reset in `SynSent`, or plan one without the `ACK` bit.

---

## F21 — issue #472
Title: net-tcp: challenge acknowledgments are not throttled
Labels: enhancement, part::net
Body:
A reset inside the window and a `SYN` inside the window each set `ack_now` at `crates/net/tcp/src/connection.rs:620-624` and `crates/net/tcp/src/connection.rs:632`, and every unacceptable segment does the same at `crates/net/tcp/src/connection.rs:610-612`. RFC 5961, section 7 (not under `docs/rfc/`) suggests a limit such as 10 challenge acknowledgments per 5 seconds; the README at `crates/net/tcp/README.md:37-41` cites RFC 5961 without the limit.

Trigger: a blind sender floods spoofed in-window resets at a connection. One acknowledgment per segment goes to the peer, which drops each one; the cost is one outgoing segment per incoming one, with no amplification.

Fix: count challenge acknowledgments per connection over a window of 5 seconds and set `ack_now` only below 10, and note the limit in the README.

---

## F22 — issue #473
Title: net-tcp: a segment that does not fit the caller's buffer loses the marks decide consumed
Labels: enhancement, part::net
Body:
`decide` takes `reset_pending` at `crates/net/tcp/src/connection.rs:867`, clears `resend_syn` at `crates/net/tcp/src/connection.rs:934`, clears `fast_retransmit` at `crates/net/tcp/src/connection.rs:974`, and counts and backs off a retransmission at `crates/net/tcp/src/connection.rs:888-903` before `transmit` writes. `Segment::write` fails for a buffer with less room than `wire_len` at `crates/net/tcp/src/segment.rs:304-310`, and `transmit` returns `None` at `crates/net/tcp/src/connection.rs:1136` without restoring any mark.

Trigger: a caller polls with a buffer shorter than a data segment of `max_segment` bytes. The retransmission is counted against `retransmit_limit`, the timer doubles, the fast retransmit is forgotten, and a pending reset is dropped; `poll_at` reports work and `poll` produces nothing.

Fix: check `buffer.len()` against the plan's wire length at the top of `poll` and return `None` before `decide` runs; the option not taken, restoring each mark in `transmit`, has to undo the timer and the attempt count.

---

## F23 — issue #474
Title: net-tcp: the default segment size over IPv6 is 536 instead of 1220
Labels: enhancement, part::net
Body:
`DEFAULT_MAX_SEGMENT` at `crates/net/tcp/src/connection.rs:37-39` is 536 for both families and `accept_peer` applies it when no option arrived at `crates/net/tcp/src/connection.rs:1232-1236`. RFC 9293, section 3.7.1 (`docs/rfc/rfc9293.txt:1740-1742`, MUST-15) assumes 536 for IPv4 and 1220 for IPv6.

Trigger: an IPv6 peer sends a `SYN` without the option. This end sends 536-byte segments where 1220 is allowed; nothing breaks, the segment count more than doubles.

Fix: choose the default by `self.local.address.version()` in `accept_peer`.

---

## F24 — issue #475
Title: net-tcp: Congestion stores the recovery point that nothing reads
Labels: enhancement, part::net
Body:
`recovering_until` at `crates/net/tcp/src/congestion.rs:57-59` is set to `highest` at `crates/net/tcp/src/congestion.rs:165` and read only through `is_some` at `crates/net/tcp/src/congestion.rs:96`, `crates/net/tcp/src/congestion.rs:117` and `crates/net/tcp/src/congestion.rs:152`. The `highest` parameter of `on_duplicate_ack` at `crates/net/tcp/src/congestion.rs:150` and `Connection::snd_max` at `crates/net/tcp/src/connection.rs:1330-1332` exist to feed it. Reno recovery ends on any new acknowledgment (`crates/net/tcp/src/congestion.rs:117-124`), which is right for RFC 5681, section 3.2; the stored number is the NewReno recovery point of RFC 6582, which the crate does not implement.

Trigger: none in behavior.

Fix: replace the field with a `bool`, drop the parameter and `Connection::snd_max`; the option not taken, implementing NewReno's partial-acknowledgment rule, is a design change beyond D-50.

---

## F25 — issue #476
Title: net-tcp: TcpError::TooLarge is constructed nowhere
Labels: enhancement, part::net
Body:
`TcpError::TooLarge` at `crates/net/tcp/src/error.rs:35-36` is built only by the display test at `crates/net/tcp/src/tests/error.rs:37`. `Segment::write` reports a short buffer as `TcpError::Wire(WireError::OutOfBounds)` at `crates/net/tcp/src/segment.rs:305-310`, and `SendBuffer::write` answers a partial count at `crates/net/tcp/src/send.rs:77-89`.

Trigger: none in behavior; a caller matching on `TooLarge` matches a value that never occurs.

Fix: remove the variant and its display arm.

---

## F26 — issue #477
Title: net-tcp: four cited RFCs are missing under docs/rfc
Labels: enhancement, part::net
Body:
The crate cites RFC 5681 (`crates/net/tcp/src/congestion.rs:4`), RFC 6298 (`crates/net/tcp/src/rto.rs:4`), RFC 5961 (`crates/net/tcp/README.md:37`) and RFC 6528 (`crates/net/tcp/src/connection.rs:389`). `docs/rfc/README.md` and `docs/rfc/fetch.sh:10-13` list neither, and `CLAUDE.md` requires every cited standard to be looked up in the concrete document under `docs/`.

Trigger: none in behavior; a reviewer of F01, F14, F17 and F21 has to fetch the documents to check the section numbers.

Fix: add the four documents to `docs/rfc/` with their entries in `README.md` and `fetch.sh`.
