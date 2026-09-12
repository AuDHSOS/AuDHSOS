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
things this system does not have: a process server that can start a
program on request, a file system that a user's keys and a session's
files live in, user accounts, and a pseudo-terminal. None of the four is
a small piece of work, and none of them is about Secure Shell. A client
presumes a TCP connection and a program that wants one, and both are
Phase 14.

What a client is for here: reaching a shell or a command on another
machine from a program of the boot archive, over a connection this system
opened and encrypted itself. It is the second thing this system can do
with a network that a person would recognise, after fetching a page.

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

## 14.3 What is there and what is missing

Everything in the left column is built and tested, with the two
qualifications the rows state.

| What SSH needs | What carries it |
|----------------|-----------------|
| X25519 | `crypto-ec::x25519`, against the vectors of RFC 7748, sections 5.2 and 6.1 |
| Finite-field Diffie-Hellman with a secret exponent | `crypto-dh` over `crypto-bignum::Modulus::pow_secret` (D-122) |
| SHA-256 | `crypto-hash::Sha256` |
| Ed25519 verification | `crypto-ec::ed25519::verify` |
| Ed25519 signing | `crypto-ec::ed25519::sign`, which compiles only under the feature `test-signing` (D-39) |
| ChaCha20 and Poly1305 as separate primitives | `crypto-aead`, which holds both beside its RFC 8439 construction |
| Unpredictable bytes | `crypto-rng` seeded from the `random_bytes` call of D-121 |
| A monotonic clock and a wait with a deadline | D-120 |
| A TCP connection with back pressure | `net-tcp`, built and tested; the socket protocol of D-116 is Phase 14, and step S8 is the one step that waits on it |

Not one new cryptographic primitive is needed. That is the result of
choosing the algorithm set in 14.5 rather than the one RFC 4253 makes
mandatory, and it is the reason this track is protocol work only.

One of them has to change what it is compiled into. D-39 keeps the
asymmetric product surface verification only and leaves signing behind
test features, and `publickey` authentication signs with the client's own
key, so step S5 needs `ed25519::sign` outside `test-signing`. That is a
decision and not code, and 14.13 carries it.

The wire types and the binary packet are built, which is step S1. What is
missing is everything above them: the negotiation, the exchange hash, the
key derivation, the authentication exchange, and the channel layer. Three
things are also missing that are not code, and 14.13 lists them.

## 14.4 The documents

All thirteen are in `docs/rfc/` under D-59, with their checksums, and
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

Three more Secure Shell documents are in the directory and are not used
by this client. RFC 6668 is the SHA-2 MACs an AEAD makes unnecessary.
RFC 5656 is the NIST-curve methods that RFC 9142 puts at SHOULD and that
add nothing this set does not already have. RFC 8332 is `rsa-sha2-256`
and `rsa-sha2-512`, which 14.5 refuses; it is kept for the asymmetry that
a later reader would otherwise have to rediscover, that the key blob of
those algorithms still names `ssh-rsa` while the signature blob does not.

The cipher is the one algorithm no standards body published, and its two
documents are in [`docs/openssh/`](openssh/README.md) rather than in
`docs/rfc/` (D-134). One is `PROTOCOL.chacha20poly1305` of the OpenSSH
source at its last revision; OpenSSH removed the file in 2025 and points
instead at the other, `draft-ietf-sshm-chacha20-poly1305-04`, which is
what the crate is written against. That the second is an Internet-Draft
is stated where it is kept, with what makes a numbered revision usable
anyway and what happens when it becomes an RFC.

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
it: `wire`, `packet`, `kex`, `auth`, `channel`, `client`.

The client is a state machine with no I/O. It is given bytes that arrived
and a buffer to write bytes into, and it answers with what it wants sent,
what it has to give its caller, and when it next has work. Everything it
needs from outside — the clock, the generator, the private key, the host
key it will accept — it is given at construction.

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
cipher block size or of eight, whichever is larger. The padding comes
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

Three things this layer does not do. There is no pty request, and so none
of the encoded terminal modes of RFC 4254, section 8: a pty is a concept
this system does not have, and asking for one on the far side without
having one on this side buys a client nothing it can use. There is no TCP
forwarding in either direction, because a forwarded channel means the
peer can make this system open connections, which is a capability
question and not a protocol one. There is no X11 forwarding.

## 14.10 Trusting a host key

This is the part with no infrastructure, and it is stated as a problem
rather than as a design because it is open (14.13).

TLS solved it with a certificate chain and the trust anchors the image
carries. SSH has no chain: a host key is trusted because it was seen
before and written down, which is a file, and this system has no writable
storage — there is no file system server, and `virtio-blk` is not
scheduled. Trust on first use needs somewhere to put the first use.

The two ends of the range are these. A fingerprint compiled into the
image is honest, checkable, and reaches only hosts that were known when
the image was built. Accepting any key on first sight is what an
interactive client does and is exactly the attack the protocol exists to
prevent, with nobody at a console to answer the question. Whatever is
chosen, the client's interface is the same: it is given the rule at
construction and never decides on its own, so that the decision lives
where it can be read.

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

**The outside check is a live OpenSSH.** The development machine runs
one; the reference machine of D-118 reaches the development machine, and
the acceptance of Phase 15 is written that way (catalog 6.6.65). The
acceptance for this track is the same shape: a program of the boot
archive opens a connection to an `sshd` the test starts, authenticates
with a key the test generated, runs a command, and reads its output and
its exit status. A handshake against a server that this project did not
write is the only evidence that the exchange hash, the key derivation
and the packet layer are what the documents mean.

**Fuzz targets** (D-23, D-54): `ssh_packet` for the binary packet reader,
and `ssh_handshake` for the `KEXINIT` name-lists and the key exchange
messages — the two places where a byte from the network chooses a length.

**The catalog.** The edge-case catalog is the definition of done (D-23);
this track's entries begin at 6.6.66 and are written with the steps that
own them.

## 14.13 What this track is waiting on

Nothing in this section is code, and none of it blocks the step it is
listed against until that step is reached.

| What is open | Where it is felt | Shape of the answer |
|--------------|------------------|---------------------|
| How a host key is trusted | step S4, and 14.10 | a rule the client is given at construction; the question is what the image can carry |
| Where the client's private key comes from | step S5 | the boot archive of D-27, or generated per boot, in which case the far side must already know the public half |
| Whether `ed25519::sign` becomes product surface | step S5, and 14.3 | a decision that amends D-39 for a client that authenticates with a key of its own, with the constant-time statement the crate makes for its other functions |

Three questions that stood here are answered. Secure Shell enters this
project as a client and this document is its design, which is D-123; the
track is track S of the roadmap, section 8.26, with the steps below; and
the cipher's documents are in `docs/openssh/`, which is D-134 and is what
step S3 was waiting for.

## 14.14 Order of work

This is track S of [the roadmap](08-roadmap.md), section 8.26.
Every step is a step of this crate unless it says otherwise, and every
one ends with `sh tools/xtask-check.sh` green, its catalog items tested,
the documents matching the code, and the changelog written — the
definition of done every phase and every track step uses.

| Step | What | Size | Ends with |
|------|------|------|-----------|
| S1 | `wire`, `packet` | M | implemented: the types of RFC 4251, section 5, encoded and decoded with the vectors of that section, and the binary packet framed, padded and read back, with the sequence numbers (catalog 6.6.68) |
| S2 | `kex` | L | `crypto-dh` is built and is the arithmetic half of this step (D-122); what remains is `SSH_MSG_KEXINIT` and the negotiation rule, both key exchange methods, the exchange hash, the six keys of section 7.2, `SSH_MSG_NEWKEYS`, and the aborts |
| S3 | the cipher | M | `chacha20-poly1305@openssh.com` over the packet layer, against the worked example of appendix A of the draft D-134 keeps |
| S4 | host keys | S-M | the `ssh-ed25519` blobs of RFC 8709, the signature over `H` verified, and the trust rule as a parameter |
| S5 | `auth` | M | `publickey` with the signature of RFC 4252, section 7, the failure and success paths, and `ext-info-c` with `server-sig-algs` |
| S6 | `channel` | L | the channel messages, the window, the session channel, `exec` and `shell`, extended data, `exit-status`, and the close sequence |
| S7 | re-exchange | S-M | a re-exchange from either side, the byte and time thresholds, and the disconnect messages with the reason codes of RFC 4250 |
| S8 | integration | M | the client over a socket of `server-net`, a program in the boot archive, and the interop acceptance of 14.12; needs Phase 14 |

S1 to S7 need nothing from another track and are built between phases, as
the whole of document 11 was. S8 is integration and needs the network on
the machine.

## 14.15 Risks

| Risk | Effect | Mitigation |
|------|--------|------------|
| No published trace to replay | the client is checked only against itself and agrees with nobody | the interop acceptance of 14.12 is not optional and is what the track is judged on; it is an end-to-end run and belongs to `test --e2e`, which `check` runs. `tools/tls-probe` is not the model for it: that one is a separate workspace and no part of the checks (11.12) |
| The `mpint` of the shared secret | a handshake that succeeds about half the time and fails otherwise, with no error that names the cause | a test for both cases — a shared secret whose top bit is set and one whose is clear — written before the exchange hash is |
| A cipher whose specification is a draft | the text the crate cites is revised or expires under it | the copy is a numbered revision and cannot change (D-134); a later revision is a later file, and the RFC it becomes goes to `docs/rfc/` with the citations moved to it |
| Trusting a host key with no storage | a client that reaches the wrong machine and cannot tell | the rule is a parameter, not a default; a client constructed without one does not connect |
| Fixed buffers meet a peer that wants more | a connection refused for a size rather than for a reason | the receive sizes are the ones RFC 4253, section 6.1, makes mandatory, so a peer that needs more than 35000 bytes is outside what it may require |
| Departing from three REQUIRED algorithms | a peer this client cannot talk to | the departure is measured, not assumed: OpenSSH negotiates every algorithm of 14.5, and the interop test is what says so on the day it stops being true |
