# audhsos-ssh audit findings

Repository: AuDHSOS/AuDHSOS. Audit of audhsos-ssh at main bc47df5.
Each `## ` section is one issue, filed 2026-09-19 with the number in its heading. `Title:` is the issue title, `Labels:` the labels, everything after `Body:` is the issue body verbatim.

---

## F01 — issue #246
Title: audhsos-ssh: the packet threshold of the re-exchange is never reset, so the client re-exchanges without end after 2^31 packets
Labels: bug, part::net
Body:
`Rekey::due` at `crates/net/ssh/src/rekey.rs:124-131` answers true when `packets >= PACKETS` (2^31). `Rekey::settled` at `crates/net/ssh/src/rekey.rs:191-199` resets `bytes` and `since` and leaves `packets` as it is, which the field doc at `crates/net/ssh/src/rekey.rs:61-63` states on purpose. `Connection::poll` at `crates/net/ssh/src/client.rs:499-502` asks for a re-exchange whenever `due` is true in state `Session`. `Rekey::note` is called for every packet in both directions at `crates/net/ssh/src/client.rs:530` and `crates/net/ssh/src/client.rs:947`.

A peer that sends 2^31 packets (28 bytes each under `chacha20-poly1305@openssh.com`, about 60 GB, which the byte threshold of one gigabyte does not stop because `bytes` resets at each exchange) drives `packets` past the threshold once and for the rest of the connection. After that, every `settled` is followed by a `due` that is true at the next `poll`, so the client sends `SSH_MSG_KEXINIT` again as soon as each `SSH_MSG_NEWKEYS` arrives: one Diffie-Hellman and one host key signature per round trip, and no channel data between them because `send` refuses outside state `Session` (F02). The reasoning at `crates/net/ssh/src/rekey.rs:25-28` is also wrong: a nonce repeats under one key only when more than 2^32 packets are sent under that key, and a sequence number wrapping between two exchanges under two different keys repeats no nonce.

Fix: count the packets since the last exchange and reset that count in `settled`, keeping the threshold at 2^31; the option of keeping the total and comparing it against the count at the last exchange costs a second field for the same result.

---

## F02 — issue #247
Title: audhsos-ssh: `send` writes channel data after the client's own `SSH_MSG_KEXINIT` and refuses with `Channel` during the rest of the exchange
Labels: bug, part::net
Body:
`Connection::send` at `crates/net/ssh/src/client.rs:406-423` checks only `state == State::Session` and never `Rekey::may_send` (`crates/net/ssh/src/rekey.rs:208-210`), which the client calls for `SSH_MSG_REQUEST_FAILURE` alone (`crates/net/ssh/src/client.rs:827` and `crates/net/ssh/src/client.rs:837`). When `poll` asks for a re-exchange at `crates/net/ssh/src/client.rs:499-502`, the state stays `Session` with `rekey` in `Asked`, so `send` frames `SSH_MSG_CHANNEL_DATA` (94) after the client's `SSH_MSG_KEXINIT` and before its `SSH_MSG_NEWKEYS`. RFC 4253, section 7.1 (`docs/rfc/rfc4253.txt:1050-1053`) forbids every message above 49 in that interval, and `crates/net/ssh/README.md:192-193` claims the crate keeps the channels off the wire while an exchange runs.

Trigger: a connection in state `Session` at `now >= since + MICROSECONDS`, `poll(now)` (sends `SSH_MSG_KEXINIT`), then `send(data)`. OpenSSH's `kex_protocol_error` answers such a packet with `SSH_MSG_UNIMPLEMENTED`, which `handle` at `crates/net/ssh/src/client.rs:569-574` ignores, so the data is lost while `Channel::write_data_message` at `crates/net/ssh/src/channel.rs:426` has spent the window for it. Once the peer's `SSH_MSG_KEXINIT` arrives the state is `KexReply` and then `NewKeys`, and `send` returns `Err(SshError::Channel)` although the command is running, which the doc at `crates/net/ssh/src/client.rs:402-405` describes as "the command is not running"; a caller that treats the error as final ends a working session over a routine re-exchange.

Fix: in `send`, return `Ok(0)` while `rekey.is_running()` and while the state is `KexReply` or `NewKeys`, and let the caller retry after `poll` reports the new keys; the option of queueing the data inside the crate costs a buffer the crate does not have.

---

## F03 — issue #248
Title: audhsos-ssh: `finish`, `close`, `grant` and the close reply write channel messages while a key exchange runs
Labels: bug, part::net
Body:
`Connection::finish` at `crates/net/ssh/src/client.rs:431-440` emits `SSH_MSG_CHANNEL_EOF` (96), `Connection::close` at `crates/net/ssh/src/client.rs:450-467` emits `SSH_MSG_CHANNEL_CLOSE` (97), `grant` at `crates/net/ssh/src/client.rs:900-910` emits `SSH_MSG_CHANNEL_WINDOW_ADJUST` (93), and `take_session` at `crates/net/ssh/src/client.rs:858-865` emits the close reply. None of the four asks `Rekey::may_send`, while `take_global_request` at `crates/net/ssh/src/client.rs:824-832` defers its `SSH_MSG_REQUEST_FAILURE` for the reason RFC 4253, section 7.1 (`docs/rfc/rfc4253.txt:1050-1053`) gives.

Trigger for `grant`: the client asks for a re-exchange in `poll`, the peer's channel data that was already in flight arrives in state `Session`, `take_session` at `crates/net/ssh/src/client.rs:853` calls `grant`, and the adjust goes out between the client's `SSH_MSG_KEXINIT` and `SSH_MSG_NEWKEYS`. OpenSSH answers with `SSH_MSG_UNIMPLEMENTED` and drops the adjust, so the peer's window stays at half while `local_window` at `crates/net/ssh/src/channel.rs:241` says it is full; the peer stops sending at a point the client does not expect. Trigger for `finish` and `close`: the caller calls either while the state is `KexReply` or `NewKeys`; `write_eof` and `write_close` at `crates/net/ssh/src/channel.rs:197-220` succeed because the channel is open, and the message goes out in the forbidden interval.

Fix: hold each of the four messages as owed, the way `owed_failures` at `crates/net/ssh/src/client.rs:250-254` holds the global answer, and emit them from `answer_owed_requests` once `settled` has run; the option of returning an error from `finish` and `close` during an exchange makes the caller retry what the crate can remember.

---

## F04 — issue #249
Title: audhsos-ssh: a command longer than 336 bytes fails with `OutOfBounds` after authentication
Labels: bug, part::net
Body:
`take_channel` at `crates/net/ssh/src/client.rs:784-788` writes the `exec` request into `[0u8; MAX_CONTROL]`, and `MAX_CONTROL` at `crates/net/ssh/src/client.rs:69` is 354 bytes (`MAX_SIGNED` 207 from `crates/net/ssh/src/auth.rs:308-333`, plus `SIGNATURE_BLOB_LEN` 83, plus 64). `Channel::write_exec` at `crates/net/ssh/src/channel.rs:250-259` needs 18 bytes before the command. `Config::command` at `crates/net/ssh/src/client.rs:122-123` states no bound, and `Connection::new` at `crates/net/ssh/src/client.rs:289-307` checks the user name and the buffers and not the command.

Trigger: `Config { command: Some(&[b'a'; 337]), .. }`. The connection finishes the key exchange and the authentication, receives `SSH_MSG_CHANNEL_OPEN_CONFIRMATION`, and `poll` returns `Err(SshError::OutOfBounds { needed: 355, available: 354 })` with the channel open on the server and no `SSH_MSG_CHANNEL_CLOSE` or `SSH_MSG_DISCONNECT` sent.

Fix: define `MAX_COMMAND`, refuse a longer command in `Connection::new` with `OutOfBounds`, and size the buffer in `take_channel` to `MAX_COMMAND + 18`; the option of writing the request straight into the outgoing buffer avoids the constant but needs the packet header framed around a payload written in place, which `Encoder::encode` does not do.

---

## F05 — issue #251
Title: audhsos-ssh: `send` documents a bound by the peer's maximum packet size and does not apply it
Labels: bug, part::net
Body:
The doc of `Connection::send` at `crates/net/ssh/src/client.rs:398-400` says the bytes taken are bounded "by the peer's maximum packet size". The computation at `crates/net/ssh/src/client.rs:410-412` takes the minimum of the data length, the remote window, `MAX_SEND` and the room, and not of `remote_max_packet`. `Channel::write_data_message` at `crates/net/ssh/src/channel.rs:412-415` then returns `Err(SshError::Window)` when the taken length exceeds `remote_max_packet`.

Trigger: a peer whose `SSH_MSG_CHANNEL_OPEN_CONFIRMATION` carries a maximum packet size of 1024 (RFC 4254, section 5.1, allows any value) and `send` with 2048 bytes: `take` is 2048 and the call fails with `Window` instead of taking 1024 bytes.

Fix: include `usize::try_from(self.machine.channel.remote_max_packet())` in the minimum at `crates/net/ssh/src/client.rs:412`, which needs a `remote_max_packet` accessor on `Channel` beside `remote_window` at `crates/net/ssh/src/channel.rs:134-138`.

---

## F06 — issue #253
Title: audhsos-ssh: a re-exchange the server starts before the session is open is refused as a wrong message
Labels: bug, part::net
Body:
`handle` at `crates/net/ssh/src/client.rs:578-580` routes `SSH_MSG_KEXINIT` to `take_kexinit` only in state `Session`. In state `ServiceAccept` the packet reaches `read_service_accept` at `crates/net/ssh/src/auth.rs:414-421`, in `UserAuth` it reaches `Response::read` at `crates/net/ssh/src/auth.rs:557-575`, and in `ChannelOpen` it reaches `Message::read` at `crates/net/ssh/src/channel.rs:525-552`; each returns `Err(SshError::Message(20))`. RFC 4253, section 9 (`docs/rfc/rfc4253.txt:1280-1284`) lets either party start a re-exchange whenever none is running.

Trigger: a server that sends `SSH_MSG_KEXINIT` after `SSH_MSG_USERAUTH_SUCCESS` and before the client's `SSH_MSG_CHANNEL_OPEN` reaches it. `poll` returns `Err(SshError::Message(20))` and the connection ends.

Fix: route `SSH_MSG_KEXINIT` to `take_kexinit` in every state from `ServiceAccept` on, and have `take_new_keys` at `crates/net/ssh/src/client.rs:707-716` return to the state that was current when the exchange began, kept in a field, instead of choosing between `Session` and `ServiceAccept` by `channel.is_open()`.

---

## F07 — issue #255
Title: audhsos-ssh: a close received while the request is unanswered is never answered and the connection never ends
Labels: bug, part::net
Body:
`take_channel` at `crates/net/ssh/src/client.rs:780-802` acts on `Report::Opened`, `Report::Succeeded` and `Report::Refused`, and maps every other report to `Ok(None)` at `crates/net/ssh/src/client.rs:800`. `Channel::apply` at `crates/net/ssh/src/channel.rs:343-346` has already recorded the peer's close. No `SSH_MSG_CHANNEL_CLOSE` is written back, which RFC 4254, section 5.3 (`docs/rfc/rfc4254.txt:470-473`) makes a MUST, and the state stays `ChannelOpen`.

Trigger: a server that answers the `exec` request with `SSH_MSG_CHANNEL_EOF` and `SSH_MSG_CHANNEL_CLOSE` and no `SSH_MSG_CHANNEL_SUCCESS`. Every later `poll` returns `WantsRead`; the caller waits for `Started` or `Closed` until its own timeout, and a later `SSH_MSG_CHANNEL_SUCCESS` from the same server moves the state to `Session` on a channel the peer has closed.

Fix: handle `Report::Close` in `take_channel` as `take_session` does at `crates/net/ssh/src/client.rs:858-865`: write the close reply, set `State::Closed`, return `Event::Closed`.

---

## F08 — issue #257
Title: audhsos-ssh: channel data received before the request is answered spends the window and is dropped
Labels: bug, part::net
Body:
In state `ChannelOpen`, `apply_channel` at `crates/net/ssh/src/client.rs:872-897` gives a `SSH_MSG_CHANNEL_DATA` or `SSH_MSG_CHANNEL_EXTENDED_DATA` to `Channel::apply`, whose `spend_local` at `crates/net/ssh/src/channel.rs:374-382` subtracts the length from `local_window`. `take_channel` then maps `Report::Data` to `Ok(None)` at `crates/net/ssh/src/client.rs:800`: no `Pending` is set, no `Event::Data` is returned, and `grant` is not called.

Trigger: a server that sends `SSH_MSG_CHANNEL_DATA` between `SSH_MSG_CHANNEL_OPEN_CONFIRMATION` and `SSH_MSG_CHANNEL_SUCCESS`, which RFC 4254, section 5.4, does not forbid. The bytes are lost, and a window's worth of such data leaves `local_window` at zero with no adjust sent, so the peer stops sending for the rest of the session.

Fix: refuse the message with `SshError::Channel` in `Channel::apply` at `crates/net/ssh/src/channel.rs:336-338` until a request has been answered, tracked by a flag the client sets on `Report::Succeeded`; the option of returning the data through the `Pending` path of `take_session` at `crates/net/ssh/src/client.rs:848-855` keeps the connection but hands the caller data before `Event::Started`.

---

## F09 — issue #259
Title: audhsos-ssh: a channel request that wants a reply is not answered
Labels: bug, part::net
Body:
`apply_channel` at `crates/net/ssh/src/client.rs:891-893` maps `ChannelEvent::Request { .. }` to `Report::Nothing` without reading `want_reply`, and no path writes `SSH_MSG_CHANNEL_FAILURE`. The doc of the event at `crates/net/ssh/src/channel.rs:508-509` says the request is "kept so that one that asked for a reply can be answered". RFC 4254, section 5.4 (`docs/rfc/rfc4254.txt:511-515`) has the recipient answer an unrecognized request with `SSH_MSG_CHANNEL_FAILURE` when `want_reply` is true.

Trigger: a server sends `SSH_MSG_CHANNEL_REQUEST` with type `xon-xoff` and `want_reply` true. The client answers nothing; a server that waits for the answer before it continues waits for the rest of the session.

Fix: when `want_reply` is true, write `SSH_MSG_CHANNEL_FAILURE` with the peer's channel number through the deferral of F03, since the reply is a channel message as well.

---

## F10 — issue #261
Title: audhsos-ssh: an unrecognized message number ends the connection instead of being answered with `SSH_MSG_UNIMPLEMENTED`
Labels: bug, part::net
Body:
`handle` at `crates/net/ssh/src/client.rs:559-591` names the transport numbers it ignores and passes every other number to the reader of the current state: `Message::read` at `crates/net/ssh/src/channel.rs:551`, `Response::read` at `crates/net/ssh/src/auth.rs:574`, `KexInit::read` at `crates/net/ssh/src/kex.rs:160-162`, `Reply::read` at `crates/net/ssh/src/exchange.rs:238-240` and `read_service_accept` at `crates/net/ssh/src/auth.rs:417-419` each return `Err(SshError::Message(n))`. RFC 4253, section 11.4 (`docs/rfc/rfc4253.txt:1479-1481`) requires an `SSH_MSG_UNIMPLEMENTED` carrying the sequence number and otherwise ignoring the message.

Trigger: in state `Session`, a packet whose first byte is 192 (`SSH2_MSG_PING`, which OpenSSH 9.5 and later send to a peer that advertised `ping@openssh.com`; this client does not, but the number stands for any future or private message). `poll` returns `Err(SshError::Message(192))` and the connection ends.

Fix: in `handle`, for a number the current state does not read, write `SSH_MSG_UNIMPLEMENTED` with the sequence number of that packet (`Decoder::sequence` at `crates/net/ssh/src/packet.rs:355-357` minus one, since `decode` has advanced it) and return `Ok(None)`; keep the error for a number the state does read with wrong contents.

---

## F11 — issue #263
Title: audhsos-ssh: README and document 14 claim the wrong-guess packet is ignored, and the client has no such path
Labels: bug, part::net
Body:
`kex::guess_is_wrong` at `crates/net/ssh/src/kex.rs:264-272` has no caller outside the tests. `take_kexinit` at `crates/net/ssh/src/client.rs:595-621` moves to `KexReply` without reading `first_kex_packet_follows`, and `take_kex_reply` at `crates/net/ssh/src/client.rs:625-631` reads the next packet as the method's reply. `crates/net/ssh/README.md:80-81` says "a guessed packet the peer announced is ignored unless both of its first names are what was chosen", and `docs/14-secure-shell-as-a-client.md:277-279` says "This client sends no guess and must still handle one".

Trigger: a server's `SSH_MSG_KEXINIT` with `first_kex_packet_follows` true and a first key exchange name that is not `curve25519-sha256`, followed by one packet of number 30. RFC 4253, section 7.1 (`docs/rfc/rfc4253.txt:1039-1042`) requires that packet to be silently ignored; `Reply::read` returns `Err(SshError::Message(30))` and the connection ends.

Fix: in `take_kexinit`, store `guess_is_wrong(&server, &choice)` in a `skip_next` field and have `handle` drop the next packet in state `KexReply` when the field is set; the option of removing the claim from both documents keeps the code and gives up a MUST of the section.

---

## F12 — issue #264
Title: audhsos-ssh: the server's next encryption key is held and copied outside `Secret` and one copy is never wiped
Labels: enhancement, part::net
Body:
`take_kex_reply` at `crates/net/ssh/src/client.rs:669-687` derives the server key into a local `server_key: [u8; KEY_BYTES]`, stores a copy in `machine.next_server_key: Option<[u8; KEY_BYTES]>` (`crates/net/ssh/src/client.rs:240`), and wipes `shared` and `client_key` at `crates/net/ssh/src/client.rs:684-686` but not `server_key`. `take_new_keys` at `crates/net/ssh/src/client.rs:700-706` takes the option, which writes `None` over the discriminant and leaves the 64 payload bytes in `Machine`, and wipes only the moved copy. `docs/04-safety-policy.md`, section 4.6, has the cryptography crates "hold key material in `Secret<N>`".

Consequence: the server's encryption key of the last exchange stays in the `Connection` for its lifetime and on the stack frame of `take_kex_reply` until overwritten, which is the erasure the policy calls best effort and this crate skips for one of its key copies.

Fix: make `next_server_key` an `Option<Secret<KEY_BYTES>>` and derive into it directly, so that `Drop` of `Secret` wipes both the stored value and the copy `take` moves out; wipe the local in `take_kex_reply` until then.

---

## F13 — issue #266
Title: audhsos-ssh: an error from `poll` leaves the state machine where it was and the next `poll` continues past the refused packet
Labels: enhancement, part::net
Body:
`step` at `crates/net/ssh/src/client.rs:529-531` advances `consumed` before `handle` runs, and no error path sets `State::Closed`. `take_auth` at `crates/net/ssh/src/client.rs:752` returns `Err(SshError::Authentication)` on `SSH_MSG_USERAUTH_FAILURE` and leaves the state at `UserAuth`; `take_kex_reply` at `crates/net/ssh/src/client.rs:662` returns `Err(SshError::HostKeyRejected)` and leaves `ephemeral` and the state at `KexReply`. The doc of `poll` at `crates/net/ssh/src/client.rs:487-489` does not say the connection is over after an error.

Consequence: a caller that logs an error and polls again resumes the protocol with the next packet: after `Authentication`, a following `SSH_MSG_USERAUTH_SUCCESS` opens the channel; after `HostKeyRejected`, a second reply with an admitted host key is checked against the same ephemeral pair. `app-ssh` at `crates/user/net-programs/src/bin/app_ssh.rs:244-247` ends on the first error, so the crate's own user is unaffected.

Fix: set `State::Closed` before returning any error from `poll`, and state in the doc that a `poll` that returned an error returns `Event::Closed` afterwards.

---

## F14 — issue #268
Title: audhsos-ssh: `Disconnect::write` documents "no language tag" and writes the field it was given
Labels: enhancement, part::net
Body:
The doc at `crates/net/ssh/src/msg.rs:338-339` says the method "writes one, with no language tag". The body at `crates/net/ssh/src/msg.rs:349` writes `self.language` as a string.

Consequence: a caller reading the doc expects an empty tag on the wire and gets whatever it put in `language`; `Connection::close` at `crates/net/ssh/src/client.rs:458-463` passes `b""` and is unaffected.

Fix: change the doc to say the language tag is the field, which may be empty; the option of dropping the field and always writing an empty string removes a value `read` returns.
