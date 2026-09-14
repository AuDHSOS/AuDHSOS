# 14. Secure Shell as a Client

Document 11 built a TLS client, and document 13 gave it a socket. This
document specifies the second protocol that can now stand on that socket:
an SSH-2 client. It is the same shape of work — a sans-I/O logic crate
over the cryptographic primitives of document 11 — and it is written down
separately because almost nothing of TLS carries over. SSH negotiates its
own way, frames its own packets, authenticates its user rather than only
its peer, and multiplexes a connection into channels of its own.

The one piece that already exists is the arithmetic: `crypto-dh` and the
constant-time exponentiation under it were built for this and are decided
in D-122, which also records that the crate stands in no plan. This is
the plan it was waiting for.

## 14.1 Purpose

A client, and not a server. That is the first statement of this document
because it is the one that decides its size.

A server is specified by section 6 of RFC 4254, and it presumes four
things: a process server that can start a program on request, a file
system that a user's keys and a session's files live in, user accounts,
and a pseudo-terminal. One of the four is built: `server-fs` answers
file requests over the two volumes of document 15. Each of the other
three is a piece of work of its own, and none of them is about Secure
Shell. A client presumes a TCP connection and a program that wants one,
and Phase 14 built both.

What a client is for here: reaching a shell or a command on another
machine from a program the root task starts, over a connection this
system opened and encrypted itself. It is the second thing this system
can do with a network that a person would recognise, after fetching a
page.

## 14.2 Scope

In scope:

- SSH-2 as a client, RFC 4251 to RFC 4254, with the assigned numbers of
  RFC 4250.
- The key exchange methods `curve25519-sha256` (RFC 8731) and
  `diffie-hellman-group14-sha256` (RFC 8268 over the group of RFC 3526,
  section 3).
- The host key algorithm `ssh-ed25519` (RFC 8709).
- The cipher `chacha20-poly1305@openssh.com`, which is an AEAD and
  carries its own integrity, so no separate MAC is negotiated.
- The authentication method `publickey` with an Ed25519 key
  (RFC 4252, section 7, and RFC 8709).
- Extension negotiation, `ext-info-c` and `server-sig-algs`
  (RFC 8308), which RFC 9142, section 3.5, makes a SHOULD.
- One channel type, `session`, with `exec` and `shell`, the exit status,
  and stderr as extended data.
- Key re-exchange, both when this side asks for it and when the peer
  does.

Not in scope, and each for a reason stated where it belongs below: a
server; a pseudo-terminal and the encoded terminal modes of RFC 4254,
section 8; TCP and X11 forwarding; compression; certificates; the
`keyboard-interactive`, `password`, and `hostbased` authentication
methods; agent forwarding.

## 14.3 What is there

Everything in the left column is built and tested. Nothing of this track
is missing; 14.13 lists the questions that were open and what answered
each.

| What SSH needs | What carries it |
|----------------|-----------------|
| X25519 | `crypto-ec::x25519`, against the vectors of RFC 7748, sections 5.2 and 6.1 |
| Finite-field Diffie-Hellman with a secret exponent | `crypto-dh` over `crypto-bignum::Modulus::pow_secret` (D-122) |
| SHA-256 | `crypto-hash::Sha256` |
| Ed25519 verification | `crypto-ec::ed25519::verify` |
| Ed25519 signing | `crypto-ec::ed25519::sign`, product surface since D-135, over the masked multiplication that decision added |
| ChaCha20 and Poly1305 as separate primitives | `crypto-aead`, which holds both beside its RFC 8439 construction |
| Unpredictable bytes | `crypto-rng` seeded from the `random_bytes` call of D-121 |
| A monotonic clock and a wait with a deadline | D-120 |
| A TCP connection with back pressure | `net-tcp`, built and tested, under the socket protocol of D-116, which `server-net` answers since Phase 14 |
| A volume to read a key and a rule off | `server-fs` over the scratch disk of document 15, which carries the three files of D-146 |

Not one new cryptographic primitive is needed. That is the result of
choosing the algorithm set in 14.5 rather than the one RFC 4253 makes
mandatory, and it is the reason this track is protocol work only.

One of them had to change what it is compiled into, and has. D-39 kept
the asymmetric product surface verification only and left signing behind
test features, and `publickey` authentication signs with the client's own
key. D-135 amends that for Ed25519: `sign` and `public_key` are product
surface, and because the point and scalar arithmetic of that module was
written for a verifier and branched on the bits it was given, the decision
carries the masked multiplication that signing now runs on. ECDSA signing
stays behind `test-signing`.

Every step is built. S1 to S7 are every layer of the protocol: the wire
types, the binary packet, the identification string, the negotiation,
both key exchange methods, the exchange hash, the six keys, the cipher,
the host key with the signature over the exchange hash, the
authentication exchange, the session channel, and the re-exchange. The
client of S8 is built off the phases under D-141 and is driven end to end
against a server written in the tests. The integration of S8 waited on
Phase 14 and no longer does: `app-ssh` is a program of the image, it
reaches an `sshd` through the socket of `server-net`, and the handshake
against that server is a step of `test --e2e` (14.12). The two questions
14.13 held open are answered by D-146.

## 14.4 The documents

All fourteen are in `docs/rfc/` under D-59, with their checksums, and
the README there says what each is kept for. The last two share a row
because the two documents above them defer to them for the same reason.

| Document | What is taken from it |
|----------|-----------------------|
| RFC 4251 | The wire types of section 5 and the naming rules of section 6 |
| RFC 4253 | The identification string, the binary packet, the negotiation, the exchange hash, the key derivation, re-exchange |
| RFC 4252 | The authentication framework and the `publickey` signature |
| RFC 4254 | Channels, windows, the session channel, the exit status |
| RFC 4250 | Message numbers, disconnect reason codes, channel failure codes, the name registries |
| RFC 9142 | The requirement levels that replace those of RFC 4253, and the SHOULD for extension negotiation |
| RFC 8731 | `curve25519-sha256`, and the encoding of the shared secret |
| RFC 8268 | `diffie-hellman-group14-sha256`, and the corrected range check |
| RFC 3526 | The 2048-bit MODP group, section 3 |
| RFC 8709 | The `ssh-ed25519` key and signature blobs |
| RFC 8308 | `ext-info-c`, `SSH_MSG_EXT_INFO`, `server-sig-algs` |
| RFC 8032, RFC 7748 | What RFC 8709 and RFC 8731 defer to |

Four more Secure Shell documents are in the directory and are not used
by the client on the wire. RFC 6668 is the SHA-2 MACs an AEAD makes
unnecessary. RFC 5656 is the NIST-curve methods that RFC 9142 puts at
SHOULD and that add nothing this set does not already have. RFC 8332 is
`rsa-sha2-256` and `rsa-sha2-512`, which 14.5 refuses; it is kept for the
asymmetry that a later reader would otherwise have to rediscover, that
the key blob of those algorithms still names `ssh-rsa` while the
signature blob does not. RFC 9987 is the agent protocol, which 14.2 puts
out of scope; its section 5.2.3 is kept because the private key format
below defers to it for the encoding of the key itself.

The cipher is the one algorithm no standards body published, and its two
documents are in [`docs/openssh/`](openssh/README.md) rather than in
`docs/rfc/` (D-134). One is `PROTOCOL.chacha20poly1305` of the OpenSSH
source at its last revision; OpenSSH removed the file in 2025 and points
instead at the other, `draft-ietf-sshm-chacha20-poly1305-04`, which is
what the crate is written against. That the second is an Internet-Draft
is stated where it is kept, with what makes a numbered revision usable
anyway and what happens when it becomes an RFC.

`PROTOCOL.key` is in that directory for the same reason: the file a
private key is stored in is OpenSSH's format and no standards body wrote
it down. Step S5 signs with a key of this client's own, and 14.13 holds
the question of where that key comes from; every answer to it reads a
file in this format, so the document is kept before the step needs it.

## 14.5 The algorithms this client offers

| Role | Name | Document | Primitive |
|------|------|----------|-----------|
| Key exchange | `curve25519-sha256` | RFC 8731 | `crypto-ec::x25519`, SHA-256 |
| Key exchange | `diffie-hellman-group14-sha256` | RFC 8268, RFC 3526 §3 | `crypto-dh`, SHA-256 |
| Host key | `ssh-ed25519` | RFC 8709, RFC 8032 | `crypto-ec::ed25519::verify` |
| Cipher and integrity | `chacha20-poly1305@openssh.com` | OpenSSH, `draft-ietf-sshm-chacha20-poly1305-04` (D-134) | `crypto-aead` |
| Compression | `none` | RFC 4253 §6.2 | — |
| Authentication | `publickey` with `ssh-ed25519` | RFC 4252 §7, RFC 8709 | `crypto-ec::ed25519::sign` |
| Extension | `ext-info-c`, `server-sig-algs` | RFC 8308 | — |

Both key exchange methods are offered, `curve25519-sha256` first. RFC 9142,
table 12, makes `diffie-hellman-group14-sha256` the one MUST and
`curve25519-sha256` a SHOULD; offering both meets the requirement and
still prefers the curve.

What is refused, and why. The table is written out because an omission
that is not written down reads as a gap, which is the form D-114 uses for
the virtio features.

| Refused | Why |
|---------|-----|
| `ssh-dss` | DSA is not implemented and will not be; RFC 9142's successor table has no place for it |
| `ssh-rsa` | RSA with SHA-1. `crypto-hash` has no SHA-1 and is not getting one |
| `rsa-sha2-256`, `rsa-sha2-512` | Sound, and unnecessary: one host key algorithm is enough for a client, and Ed25519 is the smaller one to get right |
| `ecdsa-sha2-*`, `ecdh-sha2-*` | RFC 9142 puts both at SHOULD; the curve methods above cover the same ground with primitives already in use |
| `3des-cbc`, every `*-cbc` | No CBC mode exists in `crypto-aead`, and these need the construction of RFC 4253, section 6, in which the packet length is encrypted and the tag cannot be checked before it is decrypted. `docs/rfc/README.md` records where that circle is written down and why the document that names it is not kept here |
| `aes*-ctr` | Would need a CTR mode and a separate MAC, and with it the MAC-then-encrypt construction over an encrypted length |
| `hmac-sha1`, `hmac-md5`, and the `-96` forms | SHA-1 and MD5 are not implemented |
| `diffie-hellman-group1-sha1` | RFC 9142, table 12: SHOULD NOT |
| `diffie-hellman-group14-sha1` | RFC 9142, table 12: MAY, and it needs SHA-1 |
| `diffie-hellman-group-exchange-*` | Nothing here negotiates a group |
| `zlib`, `zlib@openssh.com` | `audhsos-deflate` compresses in one call over one block; SSH compression is one stream across a connection, flushed at every packet, with the window carried between them. That is a second mode of that crate, not a parameter of this one |
| `none` as a cipher or a MAC | An unencrypted connection is not a thing this client can be talked into |

One thing this client does not send is a MAC list. RFC 4253, section 7.1,
asks every algorithm list to hold at least one name, and the one cipher
offered is an AEAD that needs no MAC, so the two MAC lists go out empty.
The alternative is naming a MAC this client does not have, which would be
a worse answer to the same sentence, and a server that would need one
cannot negotiate with this client anyway: it offers exactly one cipher.

Three of these — `ssh-dss`, `3des-cbc`, `hmac-sha1` — are REQUIRED in
RFC 4253. RFC 9142 withdrew the two SHA-1 key exchanges of the same
document but not these; refusing them is a departure from the standard,
it is deliberate, and it is stated here rather than left to be discovered.
The client interoperates with OpenSSH, which is what the departure is
measured against in 14.12.

## 14.6 The crate

One crate, `audhsos-ssh` at `crates/net/ssh`, standing to this protocol
as `audhsos-tls` stands to TLS: sans-I/O under D-49, no allocation, time
and randomness as parameters, host-tested. It is layer c3 of the catalog
in document 5, above `crypto-dh`, `crypto-ec`, `crypto-aead`,
`crypto-hash`, `crypto-rng` and `crypto-ct`, and below nothing.

Three RFCs and not three crates. The transport, the authentication and
the connection layer are one specification cut into three files, and they
share what a crate boundary would have to hand back and forth: the packet
writer, the sequence numbers, and the session identifier that the
authentication signature is over. `audhsos-tls` is one crate with
`record`, `handshake`, `keys` and `client` as modules, and this follows
it: `wire`, `packet`, `ident`, `msg`, `kex`, `exchange`, `keys`,
`cipher`, `hostkey`, `auth`, `channel`, `rekey`, `client`. The small ones carry what every
layer above them cites: `ident` is the identification string of RFC 4253,
section 4.2, which is neither a packet nor a key exchange and goes into
the exchange hash of both, and `msg` is the message numbers of RFC 4250.
`kex` is the negotiation, `exchange` the two methods and the exchange
hash, `keys` the derivation of section 7.2, and `cipher` the one cipher.

The client is a state machine with no I/O. It is given bytes that arrived
and a buffer to write bytes into, and it answers with what it wants sent,
what it has to give its caller, and when it next has work. Everything it
needs from outside — the clock, the generator, the private key, the host
key it will accept — it is given at construction.

**The program that uses it** is `app-ssh`, a binary of
`user-net-programs` beside `server-net` and `app-net` (D-144). It reads
the port, the account, the command, the trust rule and its own secret off
the scratch volume (D-146), opens a `user_programs::socket::Stream`, and
moves bytes between that stream and the crate: what `write_ssh` gives it
goes to the stream, what the stream answers goes to `read_ssh`, and the
events of `poll` are what it reports. It decides nothing about the
protocol and nothing about the transport — every refusal it reports comes
from the crate, and it reaches no ring and holds no `unsafe` block. A
boot whose volume carries no configuration — which is every boot but the
interop one — is one line and an end.

## 14.7 The transport layer

**The identification string** (RFC 4253, section 4.2): `SSH-2.0-` and a
software version, terminated by CR LF, at most 255 characters including
those two. The server may send other lines first, and they must not begin
with `SSH-`. The part of both strings before the CR LF goes into the
exchange hash, so both are kept after they are parsed; this is the first
place an implementation loses a handshake by discarding what it has
already read.

**The binary packet** (RFC 4253, section 6): a `uint32` length, a padding
length byte, the payload, at least four bytes of random padding, and the
integrity tag. The length of everything but the tag is a multiple of the
cipher block size or of eight, whichever is larger — except that the
cipher of 14.5 encrypts the length field with a key of its own and leaves
it outside the region the padding aligns. Neither document that describes
that cipher says so in words; its worked example does, and it is a packet
of 76 bytes whose length field names 72. The padding comes
from the generator this crate is given, once per packet; nothing calls
`random_bytes` per packet, which is what D-121 requires.

**The sequence number** (RFC 4253, section 6.4) is a `uint32` that never
appears on the wire, starts at zero, is not reset by a re-exchange, and
wraps at 2^32. It is an input to the AEAD and it is what makes a replayed
packet fail.

**Negotiation** (RFC 4253, section 7.1): `SSH_MSG_KEXINIT` with a cookie
and ten name-lists. For the cipher, the integrity and the compression
lists the rule is the first name on the client's list that the server
also has. For the key exchange it is not only that: the document iterates
the client's list and takes the first method the server also has *and*
for which a host key algorithm both sides have satisfies what that method
needs of it, so the key exchange and the host key are chosen together.
A peer may send a guessed first key exchange packet, and if the guess was
wrong that packet is *silently ignored* and is not an error. This client
sends no guess and must still handle one.

**The exchange hash** (RFC 4253, section 8):
`H = SHA-256(V_C || V_S || I_C || I_S || K_S || e || f || K)`, the first
five as `string` and the last three — for the curve method, per RFC 8731,
section 3 — as they are defined there. `H` of the *first* exchange
becomes the session identifier and never changes afterwards.

**The shared secret is an `mpint`** (RFC 8731, section 3.1), and this is
the trap worth naming twice. X25519 gives 32 bytes that RFC 7748 defines
little-endian; SSH reads those same bytes as a big-endian unsigned
integer and then encodes that integer under section 5 of RFC 4251 — so a
leading zero byte appears when the top bit is set and the value is
stripped of leading zeros when it is not. A client that feeds the 32
bytes in as a fixed-length string computes a different hash from its peer
on about half of its connections and succeeds on the rest. The
finite-field method has the same rule and the same trap.

**The keys** (RFC 4253, section 7.2): six of them, `HASH(K || H || X ||
session_id)` for `X` in `A` to `F`, extended by hashing `K || H || <what
there is so far>` until there is enough. It is not HKDF and cannot be
taken from `crypto-hash::hkdf`.

**Aborts.** RFC 8731, section 3, requires a disconnect with
`SSH_DISCONNECT_KEY_EXCHANGE_FAILED` for a shared secret of all zeros and
for a public value of the wrong length. The finite-field method requires
the open interval `1 < e < p-1` of RFC 8268, section 4 — not the closed
one printed in RFC 4253, section 8 — and `crypto-dh` already enforces it
on both the received value and the sent one.

**Re-exchange** (RFC 4253, section 9): either side may start one, the
roles do not change, the session identifier does not change, and the
contexts are reset. This client asks for one after a gigabyte or an hour,
whichever comes first, which is the document's recommendation, and it
must ask before the sequence number wraps. Channel traffic does not stop
while it runs; what stops is everything that is not a transport message.

## 14.8 The authentication layer

The framework of RFC 4252, section 4: a request naming a user, a service
and a method, answered by a failure that lists what may still be tried
and carries a partial-success flag, or by a success that ends
authentication for the connection and is sent once.

This client sends `publickey` with an Ed25519 key. The signature of
section 7 is over the session identifier followed by the request fields,
which is what makes a signature captured from one connection worthless on
another. The client may query first, with the boolean false and no
signature, and take `SSH_MSG_USERAUTH_PK_OK` as leave to sign; it does
both, because the query costs one round trip and the signature costs a
private key operation, and because a server that answers the query has
told the client its key is acceptable.

`ext-info-c` goes into the `kex_algorithms` list of the first
`SSH_MSG_KEXINIT`, per RFC 8308, section 2.1, where it cannot be chosen
as a key exchange because the server's spelling differs. The
`server-sig-algs` that may come back is read and remembered. With one
host key algorithm and one authentication algorithm this client has
nothing to choose from, so the extension changes no behaviour today; it
is implemented because RFC 9142, section 3.5, makes it a SHOULD and
because the alternative is discovering later that the negotiation has no
place to put it.

What a server does with a user name it does not know is stated in
RFC 4252, section 5: it may disconnect, or it may answer with a list of
methods that cannot succeed, so that the answer does not say which
accounts exist. A client cannot distinguish the second from a real
failure list and must not try.

## 14.9 The connection layer

One channel type, `session`, and the messages of RFC 4254, sections 5 and
6.

A channel is opened with a local number, an initial window, and a maximum
packet size (section 5.1). The window is a credit the sender spends and
the receiver grants back with `SSH_MSG_CHANNEL_WINDOW_ADJUST`, up to
2^32 - 1 and never past it (section 5.2). Extended data — stderr, type 1
— spends the same window as ordinary data, which is the detail that a
second buffer with a window of its own would get wrong.

On the session channel: `exec` for a command and `shell` for a login
shell, the environment requests a server is free to ignore, `exit-status`
and `exit-signal` on the way out, and `eof` and `close` under the rule of
section 5.3: a close may arrive with no eof before it, a close must be
answered with a close unless one was already sent, and the channel is
closed for a party only when it has both sent and received one.

A request the peer makes of the connection and not of a channel
(section 4) is answered and not acted on: this client offers no
forwarding, no agent and no host key proof, so every name is one it does
not recognise, and section 4 answers that with `SSH_MSG_REQUEST_FAILURE`
where a reply was asked for and with nothing where it was not. OpenSSH
sends `hostkeys-00@openssh.com` as such a request as soon as it has
authenticated a client, which is what the interop run of 14.12 found.

Three things this layer does not do. There is no pty request, and so none
of the encoded terminal modes of RFC 4254, section 8: a pty is a concept
this system does not have, and asking for one on the far side without
having one on this side buys a client nothing it can use. There is no TCP
forwarding in either direction, because a forwarded channel means the
peer can make this system open connections, which is a capability
question and not a protocol one. There is no X11 forwarding.

## 14.10 Trusting a host key

SSH has no certificate chain, so a client trusts a host key because it
read that key from a source it trusts. The crate judges no key on its
own: `hostkey::Trust` is a parameter, `hostkey::accept` refuses a key no
rule admits before it checks a signature, and one place states which
hosts this system will talk to. Two rules are built.

| Rule | What it admits |
|------|----------------|
| `hostkey::Fingerprint` | one host key, by the SHA-256 of its blob, which is what OpenSSH prints after `SHA256:` |
| `hostkey::Fingerprints` | every host key of a slice of such digests; the slice is the caller's and the crate allocates nothing; an empty one admits no key |

Where the digests come from is the program's, and D-146 settles it for
the one program that exists. Two sources this system holds:

- A fingerprint in the image: costs nothing at run time, and reaches the
  hosts that were known when the image was built. It is compiled into the
  client, or it is a file of the boot volume, which this system reads and
  does not write (document 15, decision D4).
- A file on the scratch disk: `server-fs` creates, reads, writes and
  removes files, and a file written in one boot is found in the next with
  the same bytes (document 15, step S8, test 3), which is what trust on
  first use needs of a volume. Two conditions come with it: a run carries
  that disk only when it asks (D-136), and `server-fs` has to be in the
  boot set before a client can open a path, so a client that finds no file
  system server needs a rule that wants no file. `app-ssh` reads
  `SSHTRUST.TXT` off that volume, one `SHA256:` line per host, and does
  not connect when it finds none.

A client that accepts any key on first sight admits the attack the
protocol exists to prevent, and this system has no console to ask.

## 14.11 Buffers, and the rule against allocating

RFC 4253, section 6.1, sets what every implementation must be able to
*receive*: an uncompressed payload of 32768 bytes and a total packet of
35000. Under D-49 there is no allocator here, so those are the sizes of
fixed buffers and they are what one connection costs. One receive buffer,
one send buffer, and the arithmetic of the key exchange on the stack.

The channel window this client advertises is a constant, and the maximum
packet size it advertises is bounded by what its transport will receive,
which RFC 4254, section 5.2, requires of it. One session channel per
connection, because that is what a client that runs a command needs and
because a table of channels is a table whose size would also have to be a
constant.

Where the two buffers stand is the caller's. `app-ssh` holds both on the
stack of its thread, which is sixty-four pages under D-145, so one
connection takes about a quarter of it. The maximum packet size it
advertises is the buffer it reads a channel payload into, because
`client::Connection::recv` copies what fits and drops the rest; a peer
that sends more than it advertised ends the connection rather than losing
bytes in silence.

## 14.12 Testing

The client is sans-I/O and most of it is checked on the host, in the two
forms document 11 uses: vectors from the documents, and the state machine
driven end to end against a server built in the tests.

**What can be transcribed** (D-40): the `mpint` and name-list encodings
worked out in hex in RFC 4251, section 5, which are the only published
vectors in the framework documents; appendix A of
`draft-ietf-sshm-chacha20-poly1305-04`, which is one whole packet — its
padding, its sequence number, the 64 bytes of key material, and the bytes
that went on the wire — and is what step S3 is checked against; the group
of RFC 3526, section 3, which `crypto-dh` already holds; and the
primitive vectors of RFC 7748, RFC 8032 and RFC 8439, which the crypto
crates already hold.

**What cannot.** There is no RFC 8448 for SSH. One packet is published
with the keys that made it and a whole handshake is not, so there is
nothing to replay and no way to check the negotiation, the exchange hash
and the key derivation against an implementation that is not itself from
a file. That is the difference between this track and the TLS track, and
it decides the shape of the outside check.

**The outside check is a live OpenSSH, and it is built.** The reference
machine reaches the development machine at the gateway of its user-mode
network (D-118), and `sh tools/xtask.sh test --e2e` runs the acceptance
last: it generates the keys of D-146 where there are none, starts an
`sshd` on a free port of the loopback with the algorithms of 14.5 named,
writes the trust file, the client's seed and the port onto a scratch disk
of its own, boots the image, and reads what `app-ssh` reports. The
command writes on both streams and exits with a status that is not zero,
so a client that reads only the first stream and one that reports no
status both fail. `sh tools/xtask.sh test --ssh` runs that one run alone.
A handshake against a server this project did not write is the only
evidence that the exchange hash, the key derivation and the packet layer
are what the documents mean, and it has already earned its place: the
client refused the global request of 14.9 until a live OpenSSH sent
one.

The run needs an `sshd` on the development machine — `/usr/sbin/sshd`,
which Debian and Ubuntu package as `openssh-server`. It runs as the
account that started the check and authenticates that same account, so
nothing here asks for a password and nothing runs as root. The scratch
disk it writes is a FAT32 volume the host made, which `server-fs` mounts
and does not format, and it is a disk of its own so that the end-to-end
run keeps the blank one its persistence test needs (D-136).

**Fuzz targets** (D-23, D-54), both built: `ssh_packet` for the binary
packet reader, with the cipher in use and without it, and `ssh_handshake`
for the identification string, the `KEXINIT` name-lists, the key exchange
messages and the host key blob — the places where a byte from the network
chooses a length.

**The catalog.** The edge-case catalog is the definition of done (D-23);
this track's entries begin at 6.6.66, end at 6.6.80, and are written with
the steps that own them.

## 14.13 What this track was waiting on

Nothing. Every question this section held is answered, and every step of
14.14 is built.

| What was open | What answered it |
|---------------|------------------|
| Whether `ed25519::sign` becomes product surface | D-135: it does, with `public_key` and with the masked point and scalar multiplications a secret scalar needs, because the arithmetic under the gate had been written for a verifier; ECDSA signing stays behind `test-signing` |
| Whether Secure Shell enters this project as a client, and on what design | D-123, which is this document |
| Where the cipher's two documents are kept | D-134: `docs/openssh/`, which is what step S3 was waiting for, with `PROTOCOL.key` beside them |
| Whether the client may be built before Phase 14 | D-141: it may, because it is a state machine with no I/O and is checked against a server written in the tests |
| Which host key rule a client is given | D-146: `hostkey::Fingerprints` over the `SHA256:` lines of `SSHTRUST.TXT`, which the host writes onto the scratch disk from every public key of `keys/ssh/` |
| Where the client's private key comes from | D-146: `SSHKEY.BIN` of the same disk, the thirty-two octets of the seed, which the xtask reads out of the `PROTOCOL.key` file `ssh-keygen` wrote |

## 14.14 Order of work

This is track S of [the roadmap](08-roadmap.md), section 8.26, and every
step of it is built. A step is a step of this crate unless it says
otherwise, and each ended with `sh tools/xtask-check.sh` green, its
catalog items tested, the documents matching the code, and the changelog
written — the definition of done every phase and every track step uses.

| Step | What | Size | Ends with |
|------|------|------|-----------|
| S1 | `wire`, `packet` | M | implemented: the types of RFC 4251, section 5, encoded and decoded with the vectors of that section, and the binary packet framed, padded and read back, with the sequence numbers (catalog 6.6.68) |
| S2 | `kex` | L | implemented: the identification string, the message numbers, `SSH_MSG_KEXINIT` and the negotiation rule (catalog 6.6.69); both key exchange methods over `crypto-dh` (D-122) and `crypto-ec::x25519`, the exchange hash, the six keys of section 7.2, `SSH_MSG_NEWKEYS`, and the aborts (catalog 6.6.70) |
| S3 | the cipher | M | implemented: `chacha20-poly1305@openssh.com` over the packet layer, against the worked example of appendix A of the draft D-134 keeps (catalog 6.6.70) |
| S4 | host keys | S-M | implemented: the `ssh-ed25519` blobs of RFC 8709, sections 4 and 6, the signature over `H` verified, the fingerprint of a blob, and the trust rule as a parameter (catalog 6.6.75) |
| S5 | `auth` | M | implemented: the service request, `publickey` with the signature of RFC 4252, section 7, the failure, success, banner and `SSH_MSG_USERAUTH_PK_OK` answers, and the `SSH_MSG_EXT_INFO` that carries `server-sig-algs` (catalog 6.6.76) |
| S6 | `channel` | L | implemented: the channel messages, the window in both directions, the session channel, `exec`, `shell` and `env`, extended data, `exit-status` and `exit-signal`, and the close sequence (catalog 6.6.77) |
| S7 | re-exchange | S-M | implemented: a re-exchange from either side, the byte, time and sequence number thresholds, what may be sent while one runs, and the disconnect message with the reason codes of RFC 4250 (catalog 6.6.78) |
| S8 | the client, and its integration | M | implemented: the state machine of 14.6 over every layer below it, driven end to end against a server written in the tests (catalog 6.6.79) and, since Phase 14, against a live OpenSSH through the socket of `server-net`, from the program `app-ssh` of the image, on the key material of D-146 (catalog 6.6.80) |

S1 to S7 needed nothing from another track and were built between phases,
as the whole of document 11 was. The client of S8 needed nothing either,
because it is logic over the layers below it and reads no socket, which
is what D-141 admits; its integration needed the network on the machine,
and Phase 14 built that.

## 14.15 Risks

| Risk | Effect | Mitigation |
|------|--------|------------|
| No published trace to replay | the client is checked only against itself and agrees with nobody | the interop acceptance of 14.12 is a step of `test --e2e`, which `check` runs, and it is what the track is judged on. `tools/tls-probe` is not the model for it: that one is a separate workspace and no part of the checks (11.12) |
| The acceptance needs an `sshd` on the machine that runs the check | a check that is red for a missing package and not for a fault | the run says which package it wants and where it looked; the workflow of `.github/workflows/ci.yml` installs `openssh-server` |
| OpenSSH drops an algorithm of 14.5 | an acceptance that passes on something other than the set this client offers | the server of the run names every algorithm of 14.5 in its configuration rather than letting the two sides prefer; an OpenSSH that has dropped one refuses to start the run |
| The `mpint` of the shared secret | a handshake that succeeds about half the time and fails otherwise, with no error that names the cause | a test for both cases — a shared secret whose top bit is set and one whose is clear — written before the exchange hash is |
| A cipher whose specification is a draft | the text the crate cites is revised or expires under it | the copy is a numbered revision and cannot change (D-134); a later revision is a later file, and the RFC it becomes goes to `docs/rfc/` with the citations moved to it |
| Trusting a host key by no rule | a client that reaches the wrong machine and cannot tell | the rule is a parameter, not a default; a client constructed without one does not connect, and `app-ssh` that finds no trust file does not connect either |
| A key of the interop run reaching the repository | a private key in the history, which no rewrite takes back | the keys are generated per checkout into `keys/ssh/`, which `.gitignore` names, and the fixtures of the tests are built from `PROTOCOL.key` rather than embedded (D-146) |
| Fixed buffers meet a peer that wants more | a connection refused for a size rather than for a reason | the receive sizes are the ones RFC 4253, section 6.1, makes mandatory, so a peer that needs more than 35000 bytes is outside what it may require |
| Departing from three REQUIRED algorithms | a peer this client cannot talk to | the departure is measured, not assumed: OpenSSH negotiates every algorithm of 14.5, and the interop test is what says so on the day it stops being true |
