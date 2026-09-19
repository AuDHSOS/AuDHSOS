# audhsos-tls audit findings

Repository: AuDHSOS/AuDHSOS. Audit of audhsos-tls at main bc47df5.
Each `## ` section is one issue, filed 2026-09-19 with the number in its heading. `Title:` is the issue title, `Labels:` the labels, everything after `Body:` is the issue body verbatim.

---

## F01 — issue #250
Title: audhsos-tls: plaintext handshake and alert records are accepted after the handshake keys exist
Labels: bug, part::net
Body:
`Connection::step` accepts a record of type `Handshake` or `Alert` in the clear whatever the state of `machine.server_handshake`: the match at `crates/net/tls/src/client.rs:530-545` opens only `ApplicationData` records and hands every other content type through as plaintext at `crates/net/tls/src/client.rs:544`. RFC 8446 section 4.3 (`docs/rfc/rfc8446.txt:3288-3291`) and section 4.4 (`docs/rfc/rfc8446.txt:3408-3409`) place every message after the `ServerHello` under the handshake keys, and section 6 (`docs/rfc/rfc8446.txt:4714-4715`) places alerts under the current keys.

An on-path party without any key can, after the `ServerHello`, inject the seven bytes `21 03 03 00 02 01 00`: `take_alert` at `crates/net/tls/src/client.rs:655-665` sets `peer_closed` and `State::Closed`, and `poll` reports `Event::PeerClosed` with no error. The same party can inject a plaintext `EncryptedExtensions` carrying an ALPN of its choice, which `handle` at `crates/net/tls/src/client.rs:704-716` stores and feeds to the transcript; the server's `Finished` then fails, so the handshake ends with `BadSignature` instead of `UnexpectedMessage`. `step_data` at `crates/net/tls/src/client.rs:608-613` opens every record, so the window is the handshake only.

Fix: in `step`, once `machine.server_handshake` is `Some`, refuse every content type but `ChangeCipherSpec` and `ApplicationData` with `TlsError::UnexpectedMessage`, as `step_data` already does; keeping the plaintext path for the window before the `ServerHello` only.

---

## F02 — issue #252
Title: audhsos-tls: the cipher suite the server chose is not checked against the suites the caller offered
Labels: bug, part::net
Body:
`ServerHello::parse` at `crates/net/tls/src/handshake.rs:316` accepts any of the three code points the crate knows, and `take_server_hello` at `crates/net/tls/src/client.rs:791-792` stores `hello.suite` without reading `config.suites`. `handle` at `crates/net/tls/src/client.rs:700-703` passes `config` to `take_certificate` only. RFC 8446 section 4.1.3 (`docs/rfc/rfc8446.txt:1728-1731`) requires the client to abort with `illegal_parameter` when the suite was not offered.

A caller that sets `suites: &[CipherSuite::ChaCha20Poly1305Sha256]` and a server that answers `0x1301` complete the handshake under `TLS_AES_128_GCM_SHA256`; the caller's suite policy has no effect on the connection.

Fix: in `take_server_hello`, return `TlsError::IllegalParameter` when `config.suites` does not contain `hello.suite`; passing `config` into the function, which `handle` already has.

---

## F03 — issue #254
Title: audhsos-tls: the session identifier the server echoes is not compared with the one that was sent
Labels: bug, part::net
Body:
`Connection::new` draws a 32-byte `session_id` at `crates/net/tls/src/client.rs:190-196` and sends it at `crates/net/tls/src/client.rs:235-242`, but stores it nowhere in `Machine` (`crates/net/tls/src/client.rs:108-147`). `ServerHello::parse` reads `session_id` at `crates/net/tls/src/handshake.rs:315`, and no code reads `hello.session_id` afterwards. RFC 8446 section 4.1.3 (`docs/rfc/rfc8446.txt:1720-1726`) requires the client to abort with `illegal_parameter` when the echo differs.

A server that echoes an empty or a different `legacy_session_id_echo` is accepted; the handshake continues.

Fix: keep the 32 bytes in `Machine` and return `TlsError::IllegalParameter` from `take_server_hello` when `hello.session_id` differs.

---

## F04 — issue #256
Title: audhsos-tls: an ALPN answer is stored without checking that it was offered
Labels: bug, part::net
Body:
`handle` at `crates/net/tls/src/client.rs:704-712` copies `extensions.alpn` into `self.alpn` without comparing it against `config.alpn` and without checking that `config.alpn` is non-empty. RFC 8446 section 4.2 (`docs/rfc/rfc8446.txt:1988-1992`) requires `unsupported_extension` for an extension response the client did not request.

A caller with `alpn: &[]` and a server whose `EncryptedExtensions` carries `application_layer_protocol_negotiation` with `h2` gets `Connection::alpn() == Some(b"h2")`. A caller that offered `http/1.1` and a server that answers `h2` gets `Some(b"h2")` as well.

Fix: in `handle`, return `TlsError::UnexpectedExtension` when `config.alpn` is empty, and `TlsError::IllegalParameter` when `chosen` is not one of `config.alpn`.

---

## F05 — issue #258
Title: audhsos-tls: EncryptedExtensions accepts unoffered and repeated extensions while its documentation says they are refused
Labels: bug, part::net
Body:
`EncryptedExtensions::parse` at `crates/net/tls/src/handshake.rs:414-427` reads only `application_layer_protocol_negotiation` and skips every other extension type, and only ALPN is checked for repetition. The doc comment at `crates/net/tls/src/handshake.rs:401-402` says extensions the client did not offer are refused, and the module doc at `crates/net/tls/src/handshake.rs:11-13` says an extension that appears twice is refused. RFC 8446 section 4.2 (`docs/rfc/rfc8446.txt:1988-1992`) requires `unsupported_extension` for a response without a request.

An `EncryptedExtensions` carrying `key_share` (51), `pre_shared_key` (41), or two `server_name` (0) entries is accepted and hashed into the transcript.

Fix: match every extension type as `ServerHello::parse` does at `crates/net/tls/src/handshake.rs:328-359`: `server_name`, `supported_groups`, and ALPN are offered and allowed; any other type is `TlsError::UnexpectedExtension`; a repeated type is `TlsError::UnexpectedExtension`.

---

## F06 — issue #260
Title: audhsos-tls: a protected change_cipher_spec record is dropped during the handshake instead of refused
Labels: bug, part::net
Body:
In `Connection::step`, a record opened under the handshake keys yields `(kind, plaintext)` at `crates/net/tls/src/client.rs:540`, and the arm `ContentType::ChangeCipherSpec => {}` at `crates/net/tls/src/client.rs:548` catches both the plaintext compatibility record mapped at `crates/net/tls/src/client.rs:538` and an inner content type of 20. RFC 8446 section 5 (`docs/rfc/rfc8446.txt:4304-4306`) requires `unexpected_message` for a protected `change_cipher_spec` record. `step_data` refuses it at `crates/net/tls/src/client.rs:628-630`.

A server that, after the `ServerHello`, seals the byte `01` with inner type 20 under `server_handshake_traffic_secret` is not refused; the handshake goes on.

Fix: keep the plaintext case as a separate branch that returns before the `match kind`, and make the `ChangeCipherSpec` arm at `crates/net/tls/src/client.rs:548` return `TlsError::UnexpectedMessage`.

---

## F07 — issue #262
Title: audhsos-tls: a protected record with zero-length handshake or alert content is not refused
Labels: bug, part::net
Body:
RFC 8446 section 5.4 (`docs/rfc/rfc8446.txt:4635-4638`) requires `unexpected_message` for a `Handshake` or `Alert` record whose inner content is empty. `step` appends an empty plaintext at `crates/net/tls/src/client.rs:552-561` and goes on; `step_data` calls `after_handshake` with an empty slice at `crates/net/tls/src/client.rs:625-627`, whose loop at `crates/net/tls/src/client.rs:744` runs zero times; `take_alert` at `crates/net/tls/src/client.rs:656-658` answers `TlsError::Decode`, which `Alert::for_error` at `crates/net/tls/src/alert.rs:123` turns into `decode_error`.

A record whose plaintext is the single byte `16` (inner type `Handshake`) is accepted at any point after the `ServerHello`; a record whose plaintext is the single byte `15` ends the connection with `decode_error`.

Fix: in `step` and `step_data`, return `TlsError::UnexpectedMessage` when `kind` is `Handshake` or `Alert` and `plaintext` is empty, before dispatching.

---

## F08 — issue #265
Title: audhsos-tls: a post-handshake message that spans two records is dropped and the continuation is misparsed
Labels: bug, part::net
Body:
`after_handshake` at `crates/net/tls/src/client.rs:743-760` parses handshake messages out of one record's plaintext and returns `Ok(())` when `read_message` answers `None` for a partial message at `crates/net/tls/src/client.rs:744`; the partial bytes are not kept, and the handshake buffer used during the handshake at `crates/net/tls/src/client.rs:552-561` is not used after it. RFC 8446 section 5.1 (`docs/rfc/rfc8446.txt:4335-4341`) allows a handshake message to be fragmented across records.

A server that sends a `NewSessionTicket` in two records loses the first fragment silently; the second record begins with ticket bytes that `HandshakeType::from_byte` at `crates/net/tls/src/handshake.rs:170` reads as a type, so the connection ends with `unexpected_message` or, when the byte is 4 or 24, with a message whose length the ticket bytes dictate.

Fix: in `step_data`, append `Handshake` plaintext to the handshake buffer and drain complete messages from it, as `step` does; deleting `after_handshake`'s own loop.

---

## F09 — issue #267
Title: audhsos-tls: handshake bytes left over at the key change after the server's Finished are not refused
Labels: bug, part::net
Body:
RFC 8446 section 5.1 (`docs/rfc/rfc8446.txt:4342-4345`) requires `unexpected_message` when the messages preceding a key change do not align with a record boundary. `take_finished` sets `State::Connected` at `crates/net/tls/src/client.rs:993`, `drain_messages` at `crates/net/tls/src/client.rs:676-687` returns when the next message is incomplete, and `step` at `crates/net/tls/src/client.rs:566-570` never reads `handshake_len` after the drain.

A server that puts its `Finished` and three more bytes into one record gets `Event::Handshaked`; the three bytes stay in the handshake buffer for the life of the connection.

Fix: in `step`, after `drain_messages`, return `TlsError::UnexpectedMessage` when `machine.state == State::Connected` and `*handshake_len != 0`.

---

## F10 — issue #269
Title: audhsos-tls: a decrypted record whose content exceeds 2^14 bytes is not refused as record_overflow
Labels: bug, part::net
Body:
`RecordProtection::open` at `crates/net/tls/src/protection.rs:179-188` returns the plaintext up to the content type byte with no length check; a record of `MAX_CIPHERTEXT` bytes yields 16623 bytes. RFC 8446 section 5.4 (`docs/rfc/rfc8446.txt:4672-4673`) limits the encoded `TLSInnerPlaintext` to 2^14 + 1 bytes, and section 5.2 (`docs/rfc/rfc8446.txt:4505-4507`) names `record_overflow` for a record beyond its limit.

A server that sends 16623 bytes of application data in one record makes `step_data` at `crates/net/tls/src/client.rs:616-618` fail with `TlsError::BufferTooSmall` (alert `internal_error`) when `out` is exactly `MAX_PLAINTEXT`, which `recv` at `crates/net/tls/src/client.rs:379` permits, and deliver 16623 bytes when `out` is larger. The same for a `Handshake` inner type at `crates/net/tls/src/client.rs:552-561`.

Fix: in `open`, return `TlsError::RecordOverflow` when `position > MAX_PLAINTEXT`.

---

## F11 — issue #270
Title: audhsos-tls: a plaintext record longer than 2^14 bytes is accepted
Labels: bug, part::net
Body:
`record::read` at `crates/net/tls/src/record.rs:102-105` applies `MAX_CIPHERTEXT` to every content type. RFC 8446 section 5.1 (`docs/rfc/rfc8446.txt:4403-4406`) limits a `TLSPlaintext` to 2^14 bytes and requires `record_overflow` beyond it. `docs/11-cryptography-and-tls.md:441` says `record.rs` enforces the 2^14 plaintext limit; the code enforces it only when sealing, at `crates/net/tls/src/protection.rs:129-131`.

A plaintext `Handshake` record of 16640 bytes before the `ServerHello` is accepted by the record layer and fails at `crates/net/tls/src/client.rs:554-556` with `TlsError::BufferTooSmall`, alert `internal_error`, where `record_overflow` is required.

Fix: in `read`, return `TlsError::RecordOverflow` when `content_type != ContentType::ApplicationData` and `length > MAX_PLAINTEXT`.

---

## F12 — issue #273
Title: audhsos-tls: the X25519 private key and the shared secret are plain arrays that are not wiped on failure or drop
Labels: bug, part::net
Body:
`Machine.private_key` is `[u8; 32]` at `crates/net/tls/src/client.rs:118`; it is zeroed only at `crates/net/tls/src/client.rs:789` after a `ServerHello` was processed, and neither `Machine` nor `Connection` has a `Drop`. `Connection::new` fills a local `private_key` at `crates/net/tls/src/client.rs:188-192` and copies it into the machine at `crates/net/tls/src/client.rs:211`, leaving the stack copy. `shared` at `crates/net/tls/src/client.rs:787-788` is a `[u8; 32]` that is dropped without a wipe after `schedule.advance`. `docs/11-cryptography-and-tls.md:148-149` says keys live as short as possible; `Secret` wipes itself at `crates/net/tls/src/secret.rs:80-84`.

A connection that ends before the `ServerHello`, by a peer alert at `crates/net/tls/src/client.rs:549-551`, by a caller drop, or by `TlsError::RecordOverflow`, leaves the ephemeral private key in memory. The shared secret, from which every traffic key of the connection derives, stays on the stack of `take_server_hello` after every successful handshake.

Fix: hold `private_key` as `crypto_ct::Secret<32>` and copy `shared` into a `Secret` before `advance`, wiping the array; wiping `private_key` in a `Drop` for `Machine` would cover the drop but not the stack copy in `new`.

---

## F13 — issue #276
Title: audhsos-tls: send with a full outgoing buffer poisons the connection
Labels: bug, part::net
Body:
`send` at `crates/net/tls/src/client.rs:345-366` calls `fail` on every error of `write_protected`, and `RecordProtection::seal` answers `TlsError::BufferTooSmall` at `crates/net/tls/src/protection.rs:133-135` when the room left in `outgoing` is below the record. `fail` at `crates/net/tls/src/client.rs:446-469` poisons the connection and queues an `internal_error` alert.

A caller with the minimum `outgoing` of 16645 bytes that calls `send` with 16384 bytes twice without `write_tls` between the calls gets `Err(BufferTooSmall)` from the second call and every later call, and the peer gets `internal_error`; a full buffer is a flow-control state, not a protocol failure.

Fix: in `send`, return `TlsError::BufferTooSmall` without calling `fail`, so that the caller can drain `outgoing` and retry; returning `Ok(0)` would need a doc change for the meaning of zero.

---

## F14 — issue #279
Title: audhsos-tls: a chosen ALPN protocol longer than 32 bytes ends the handshake with decode_error
Labels: bug, part::net
Body:
`Machine.alpn` is `[u8; 32]` at `crates/net/tls/src/client.rs:140`, and `handle` at `crates/net/tls/src/client.rs:707` answers `TlsError::Decode` when the chosen name does not fit. `write_client_extensions` at `crates/net/tls/src/handshake.rs:259-269` offers any name up to 255 bytes.

A caller that offers a 33-byte protocol name and a server that selects it get `TlsError::Decode`, alert `decode_error`, on a well-formed message.

Fix: size `alpn` at 255 bytes, which is the largest name a `vector8` carries; storing an index into `config.alpn` instead removes the copy and, together with F04, the unoffered case.

---

## F15 — issue #282
Title: audhsos-tls: a record of only padding ends the connection with bad_record_mac instead of unexpected_message
Labels: bug, part::net
Body:
`RecordProtection::open` answers `TlsError::BadRecord` at `crates/net/tls/src/protection.rs:184` when no non-zero byte is found, `Alert::for_error` maps it to `bad_record_mac` at `crates/net/tls/src/alert.rs:122`, and the doc comment at `crates/net/tls/src/protection.rs:163-164` states that mapping. RFC 8446 section 5.4 (`docs/rfc/rfc8446.txt:4666-4669`) requires `unexpected_message`. The test at `crates/net/tls/src/tests/protection.rs:147` asserts `BadRecord`.

A record whose plaintext is two zero bytes under valid keys ends the connection with `bad_record_mac`, which tells the peer its keys are wrong when they are not.

Fix: return `TlsError::UnexpectedMessage` at `crates/net/tls/src/protection.rs:184` and update the doc comment and the test.

---

## F16 — issue #285
Title: audhsos-tls: a supported_versions answer naming another version is refused with protocol_version instead of illegal_parameter
Labels: bug, part::net
Body:
`ServerHello::parse` at `crates/net/tls/src/handshake.rs:361-363` answers `TlsError::UnsupportedVersion` both when the extension is absent and when it names a version other than `0x0304`, and `Alert::for_error` maps it to `protocol_version` at `crates/net/tls/src/alert.rs:124`. RFC 8446 section 4.2.1 (`docs/rfc/rfc8446.txt:2194-2197`) requires `illegal_parameter` when the extension is present and names a version not offered; `protocol_version` is for a server that chose a version the client does not speak, which is the absent case (Appendix D.1, `docs/rfc/rfc8446.txt:7751-7753`).

A `ServerHello` whose `supported_versions` says `0x0303` ends the connection with `protocol_version`.

Fix: return `TlsError::IllegalParameter` when `version` is `Some` and differs from `VERSION_TLS13`, and `TlsError::UnsupportedVersion` when it is `None`.

---

## F17 — issue #288
Title: audhsos-tls: an empty certificate_list is refused with bad_certificate instead of decode_error
Labels: bug, part::net
Body:
`take_certificate` at `crates/net/tls/src/client.rs:820` answers `TlsError::BadCertificate` when `entries.next()` is `None`. RFC 8446 section 4.4.2.4 (`docs/rfc/rfc8446.txt:3787-3788`) requires `decode_error` for an empty `Certificate` message.

A `Certificate` message with an empty context and an empty list ends the connection with `bad_certificate`.

Fix: return `TlsError::Decode` for the `None` case and keep `BadCertificate` for a leaf that does not parse.

---

## F18 — issue #291
Title: audhsos-tls: extensions on a certificate entry are skipped without checking that they were requested
Labels: bug, part::net
Body:
`Certificates::next` at `crates/net/tls/src/handshake.rs:484-488` reads each entry's extension vector and discards it. RFC 8446 section 4.4.2 (`docs/rfc/rfc8446.txt:3604-3605`) requires the extensions of a server's `Certificate` to correspond to ones in the `ClientHello`, and section 4.2 (`docs/rfc/rfc8446.txt:1988-1992`) names `unsupported_extension` for the other case. The `ClientHello` at `crates/net/tls/src/handshake.rs:227-281` requests no `status_request` and no `signed_certificate_timestamp`.

A `Certificate` entry carrying a `status_request` (5) extension of any content is accepted.

Fix: return `TlsError::UnexpectedExtension` from `next` when the extension vector is not empty.

---

## F19 — issue #293
Title: audhsos-tls: user_canceled is treated as an error alert
Labels: bug, part::net
Body:
`take_alert` at `crates/net/tls/src/client.rs:655-665` answers `TlsError::PeerAlert(code)` for every description but `close_notify`. RFC 8446 section 6.1 (`docs/rfc/rfc8446.txt:4832-4837`) lists `user_canceled` (90) as a closure alert that is followed by `close_notify`.

A server that sends `user_canceled` and then `close_notify` ends the connection with `Err(PeerAlert(90))` at the first alert; the caller reads a failure where the peer reported an orderly cancel.

Fix: in `take_alert`, treat code 90 as no-op and wait for the `close_notify`; the doc comment at `crates/net/tls/src/client.rs:650-654` changes with it.

---

## F20 — issue #296
Title: audhsos-tls: fail writes a plaintext alert when sealing under existing keys fails
Labels: bug, part::net
Body:
`fail` at `crates/net/tls/src/client.rs:456-465` calls `write_plain` whenever `write_protected` returns an error, which includes `TlsError::BufferTooSmall` and `TlsError::SequenceExhausted` while `client_application` or `client_handshake` is `Some`. RFC 8446 section 6 (`docs/rfc/rfc8446.txt:4714-4715`) places alerts under the current keys.

With keys installed and 10 bytes of room in `outgoing`, an error queues a plaintext alert of 7 bytes because the sealed alert of 23 bytes does not fit; the alert description is visible to a passive observer, and the peer fails to open the record. The same happens for `SequenceExhausted` with any room.

Fix: call `write_plain` only when both `client_application` and `client_handshake` are `None`.

---

## F21 — issue #300
Title: audhsos-tls: close before the handshake is done sends no close_notify and answers UnexpectedMessage
Labels: bug, part::net
Body:
`close` at `crates/net/tls/src/client.rs:397-408` seals under `client_application` only; `write_protected` at `crates/net/tls/src/client.rs:480` answers `TlsError::UnexpectedMessage` when it is `None`, and `close` sets `State::Closed` regardless. RFC 8446 section 6.1 (`docs/rfc/rfc8446.txt:4845-4846`) requires a `close_notify` before closing the write side; `fail` at `crates/net/tls/src/client.rs:451-455` already picks the handshake keys when the application keys do not exist.

A caller that calls `close` while in `WaitCertificate` gets `Err(UnexpectedMessage)`, the state is `Closed`, `poll` at `crates/net/tls/src/client.rs:316` makes no further progress, and the server sees a transport close with no alert.

Fix: choose the keys as `fail` does, and send the alert in the clear before the `ServerHello` where no keys exist.

---

## F22 — issue #303
Title: audhsos-tls: write_tls writes past the count it reports into the caller's buffer
Labels: bug, part::net
Body:
`write_tls` at `crates/net/tls/src/client.rs:299-302` zips `output` with the whole of `self.outgoing`, so it copies `min(output.len(), outgoing.len())` bytes and reports `taken = min(output.len(), outgoing_len)`. `server.rs` has the same loop at `crates/net/tls/src/server.rs:246-249`.

With `outgoing_len == 10` and an `output` of 100 bytes, bytes 10 to 99 of `output` receive stale content of `outgoing`, which is already transmitted data; a caller that keeps its own bytes after `taken` in that buffer loses them.

Fix: zip `output` with `self.outgoing[..taken]`.

---

## F23 — issue #306
Title: audhsos-tls: no key update before the AES-GCM record limit
Labels: enhancement, part::net
Body:
`take_nonce` at `crates/net/tls/src/protection.rs:197-207` serves 2^64 records per epoch, and `client.rs` sends a `KeyUpdate` only in answer to one at `crates/net/tls/src/client.rs:750-756`. RFC 8446 section 5.5 (`docs/rfc/rfc8446.txt:4692-4698`) says implementations should update keys before 2^24.5 full-size records under AES-GCM.

A connection under `TLS_AES_128_GCM_SHA256` or `TLS_AES_256_GCM_SHA384` that sends more than 2^24.5 records keeps the same key past the margin the standard names.

Fix: in `send`, call `update_client_keys` with `request_update = false` when `client_application.sequence()` reaches 2^24 and the suite is AES-GCM.

---

## F24 — issue #309
Title: audhsos-tls: a record full of KeyUpdate requests exhausts the outgoing buffer and poisons the connection
Labels: enhancement, part::net
Body:
`after_handshake` at `crates/net/tls/src/client.rs:750-756` answers every `KeyUpdate` with `request_update = 1` by a `KeyUpdate` record of 27 bytes through `update_client_keys` at `crates/net/tls/src/client.rs:1014-1029`; `write_protected` fails with `TlsError::BufferTooSmall` when `outgoing` is full, and `recv` at `crates/net/tls/src/client.rs:385-388` then calls `fail`. RFC 8446 section 4.6.3 (`docs/rfc/rfc8446.txt:4253-4262`) describes answering several `KeyUpdate` messages with a single update.

A record whose plaintext holds 617 `KeyUpdate` messages of 5 bytes each needs 16659 bytes of `outgoing`, above the minimum of 16645, and ends the connection with `internal_error`.

Fix: in `after_handshake`, set a flag when a `KeyUpdate` requests an update and send one `KeyUpdate` after the loop.

---

## F25 — issue #312
Title: audhsos-tls: handshake-stage secrets stay in the connection after the handshake
Labels: enhancement, part::net
Body:
`take_finished` at `crates/net/tls/src/client.rs:989-993` drops `client_handshake` and `server_handshake` but keeps `client_finished`, `server_finished`, and `schedule`, whose master secret at `crates/net/tls/src/keys.rs:212` no later step reads: `update_server_keys` and `update_client_keys` at `crates/net/tls/src/client.rs:998-1042` derive from `server_secret` and `client_secret` only. `docs/11-cryptography-and-tls.md:148-149` says keys live as short as possible.

Two finished keys and the master secret stay in memory for the life of the connection with no reader.

Fix: set `client_finished`, `server_finished`, and `schedule` to `None` at the end of `take_finished`.

---

## F26 — issue #317
Title: audhsos-tls: replace_with_message_hash has no caller outside the tests
Labels: enhancement, part::net
Body:
`Transcript::replace_with_message_hash` at `crates/net/tls/src/transcript.rs:65-76` is called only from `crates/net/tls/src/tests/`; `take_server_hello` refuses every `HelloRetryRequest` at `crates/net/tls/src/client.rs:778-783`, as `docs/11-cryptography-and-tls.md:485-488` records. `docs/11-cryptography-and-tls.md:451-452` lists the substitution as a function of the module.

The function is product code with no product path, and the module entry in document 11 describes a step the client does not take.

Fix: remove the function and its constant with the tests that cover them, and drop the substitution from the module list in document 11; keeping it under `#[cfg(test)]` preserves the vector test at the cost of dead product code.

---

## F27 — issue #319
Title: audhsos-tls: the recv documentation says zero means no data arrived, but zero is also the peer close
Labels: enhancement, part::net
Body:
The doc comment at `crates/net/tls/src/client.rs:368-369` says a result of zero means none had arrived. `step_data` returns `Ok(None)` at `crates/net/tls/src/client.rs:576-578` when the state is `Closed`, and at `crates/net/tls/src/client.rs:640-642` after a `close_notify`, and `recv` maps `None` to zero at `crates/net/tls/src/client.rs:383-384`.

A caller that reads the doc comment polls for more data after the peer closed; `poll` is the only call that reports `Event::PeerClosed`.

Fix: state in the doc comment that zero also follows a peer close, and that `poll` tells the two apart.

---

## F28 — issue #321
Title: audhsos-tls: the test server writes a constant ServerHello.random
Labels: enhancement, part::net
Body:
`write_server_hello` at `crates/net/tls/src/server.rs:910` writes `[0x44; 32]` as the random. RFC 8446 section 4.1.3 (`docs/rfc/rfc8446.txt:1712-1713`) requires 32 bytes from a secure generator. `ServerConfig` at `crates/net/tls/src/server.rs:78-81` takes the ephemeral key as a parameter for the same reason and documents it; the random has no parameter and no note.

The acceptance run of Phase 15 over a socket presents the same random on every connection.

Fix: add a `random: [u8; 32]` field to `ServerConfig` beside `ephemeral`, filled by the caller from its generator.
