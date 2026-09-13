# audhsos-ssh

The SSH-2 client of [document 14](../../../docs/14-secure-shell-as-a-client.md),
sans-I/O: it is given bytes that arrived and a buffer to write into, and
it never reads a socket, allocates, or asks what time it is. What exists
today is steps S1 to S7 of track S — every layer of the protocol — and
the client that drives them, which is the logic half of step S8. What
waits on the network on the machine is the socket under that client, the
program around it, and the handshake against an OpenSSH.

## `wire`

The types of RFC 4251, section 5 — `byte`, `boolean`, `uint32`,
`uint64`, `string`, `mpint`, `name-list` — as a [`wire::Reader`] and a
[`wire::Writer`] over a caller's buffer. What a reader hands out borrows
that buffer, so a packet is read where it landed and nothing is copied.
Every operation is O(n) in the bytes it moves, and a failed read leaves
the position where it was.

Two of the seven carry rules that an encoder gets wrong silently. An
`mpint` is canonical or it is refused: zero is a string of no bytes, a
positive number whose top bit is set gets one zero byte in front, and an
unnecessary leading `00` or `ff` is an error rather than a value. A
`name-list` holds names that are US-ASCII, are not empty, and hold no
comma, so an empty name — a list that begins, ends, or doubles a comma —
is an error too.

## `packet`

The binary packet of RFC 4253, section 6: a `uint32` length, the padding
length, the payload, and at least four bytes of padding, with the whole
a multiple of the cipher block size or eight, whichever is larger. The
padding comes from an [`Rng`](crypto_rng::Rng) the caller passes, once
per packet, which is what D-121 requires of everything below
`random_bytes`.

[`packet::Encoder`] frames what is sent and [`packet::Decoder`] reads
what arrives; one of each per direction, because each carries the
sequence number of section 6.4 — a `uint32` that never appears on the
wire, starts at zero, is not reset by a re-exchange, and wraps at 2^32.
A new cipher changes the block size through `set_block` and nothing
else, so there is no way to take a new block and lose the count with it.

A decoder refuses a packet before it waits for it: as soon as four bytes
have arrived, the length is held against the block size and against
[`packet::MAX_FRAME`], which is the largest packet that can satisfy both
bounds of RFC 4253, section 6.1, at once. A buffer of
[`packet::MAX_PACKET`] is therefore more than a connection ever needs.

## `ident`

The identification string of RFC 4253, section 4.2: `SSH-2.0-` and a
software version, CR LF, at most 255 characters including those two. The
part before the CR LF is what goes into the exchange hash, so it is
handed back rather than dropped. A server may send other lines first and
they may not begin with `SSH-`; those are skipped and counted. Every
line is held to the same length whether it has ended or not, so what is
refused does not depend on how the bytes were split on the way here.

## `msg`

The message numbers of RFC 4250, section 4.1.2 — the ones the layers
that exist send. The numbers specific to a key exchange method are named
with the method, because 30 to 49 mean nothing until one knows which
method is running.

## `kex`

`SSH_MSG_KEXINIT` (RFC 4253, section 7.1) and the rule that chooses from
two of them. [`kex::CLIENT`] is the algorithm set of document 14, section
14.5, with `ext-info-c` in the key exchange list (RFC 8308) and empty MAC
lists, because the one cipher offered is an AEAD that carries its own
integrity.

For the ciphers, the MACs and the compression the rule is the first name
on the client's list that the server also has. For the key exchange it is
not: the method and the host key algorithm are chosen together, because a
method that needs a signature cannot be run with a key that cannot sign.
An `ext-info-c` that ends up chosen is a disconnect and not a method
(RFC 8308, section 2.2), and a guessed packet the peer announced is
ignored unless both of its first names are what was chosen.

## `exchange`

The two methods and their two messages, and the exchange hash of RFC
4253, section 8. [`exchange::Ephemeral`] holds the scalar or the exponent
and clears it when it is dropped; the shared secret leaves in a buffer
the caller owns.

The public values are strings for `curve25519-sha256` (RFC 5656, section
4) and `mpint`s for `diffie-hellman-group14-sha256`, and the shared
secret is an `mpint` for both — which for the curve method is what RFC
8731, section 3.1, spells out, and what an implementation that hashes
thirty-two fixed bytes gets wrong on half of its connections.

The aborts are refusals and not values: a public value of the wrong
length, one outside the open interval of RFC 8268, section 4, and a
shared secret of all zeros each end the exchange where they are found.

## `keys`

The six keys of RFC 4253, section 7.2: `HASH(K || H || X || session_id)`
for `X` from `A` to `F`, extended by hashing `K || H || <the key so far>`
until there is enough. It is not HKDF, so `crypto-hash::hkdf` is the
wrong tool for it.

## `cipher`

`chacha20-poly1305@openssh.com`: two `ChaCha20` instances under one
512-bit key, one for the four-byte length field and one for the packet
and the Poly1305 key. The tag is checked before anything is decrypted.
[`packet::Encoder::set_cipher`] and [`packet::Decoder::set_cipher`] take
it into use without touching the sequence number, which is what
`SSH_MSG_NEWKEYS` needs of them.

With this cipher the length field is outside the region the padding
aligns, which the worked example of its draft shows: a packet of 76
bytes whose length field names 72.

## `hostkey`

The `ssh-ed25519` blobs of RFC 8709, sections 4 and 6, and the rule that
says which host key this client will talk to. [`hostkey::accept`] reads
`K_S`, asks the rule, and checks the signature over the exchange hash, in
that order, so a key from a host this client will not reach costs no
signature check.

SSH has no certificate chain, so the rule is a parameter and this crate
judges no key of its own (document 14, section 14.10).
[`hostkey::Fingerprint`] is the first of the two sources that section
names, a SHA-256 the image carries; the second is a file, which a caller
reads through the file system server and this crate does not.

## `auth`

RFC 4252: the service request, the `publickey` method with an Ed25519
key, and what a server answers with. [`auth::write_query`] asks whether a
key would be accepted, [`auth::write_publickey`] signs, and
[`auth::Response`] is the failure with its method list, the success, the
banner, and the `SSH_MSG_USERAUTH_PK_OK` that is leave to sign.

The signature of section 7 is over the session identifier and then the
fields of the request, so a signature captured from one connection is
worthless on another. The signed data is written once into a scratch
buffer the caller owns and the request is that data without the session
identifier, so no field is encoded twice; [`auth::signed_len`] and
[`auth::request_len`] are how long the two are.

Where the private key comes from is the caller's — [`auth::ClientKey`]
takes the secret and clears it when it is dropped — and section 14.13 of
the document holds that question open.

[`auth::ExtInfo`] reads the `SSH_MSG_EXT_INFO` of RFC 8308, section 2.3,
keeps `server-sig-algs`, and skips every other extension whatever its
value holds, which section 2.5 requires.

## `channel`

RFC 4254: one `session` channel, its window, and the requests that start
a program. [`channel::Channel`] holds the two windows and what each side
has said about the end of the channel; [`channel::Message`] is what
arrived, and [`channel::Channel::apply`] is what that message changed.

The window is a credit the sender spends and the receiver grants back
with `SSH_MSG_CHANNEL_WINDOW_ADJUST`, never past 2^32 - 1. Extended data
— stderr — spends the same window as ordinary data, which is why there is
one window here and not two. A data message is refused above the window
and above the maximum packet size the peer advertised, and a refused
write spends nothing.

The close sequence is section 5.3: a close may arrive with no end of file
before it, a close is answered with a close unless one was sent already,
and the channel is closed for this side only when it has both sent and
received one.

## `rekey`

RFC 4253, section 9: when this client asks for a re-exchange, what the
peer's `SSH_MSG_KEXINIT` asks of it, and what may be sent while one runs.
[`rekey::Rekey`] counts the bytes since the last exchange and the packets
since the connection began, and [`rekey::Rekey::due`] is given the moment
it is asked about, because no logic crate here reads a clock (D-46).

One is due after a gigabyte, after an hour, or at half the sequence
number space — the third is this crate's, because the sequence number of
section 6.4 wraps at 2^32 and a re-exchange must happen before it does.
While an exchange runs only the transport layer may send, so the
authentication and the channels wait for the new keys.

[`msg::Disconnect`] is the message of section 11.1, with the reason codes
of RFC 4250 beside it in [`msg::disconnect`].

## `client`

One state machine over every layer below it, with no I/O:
[`client::Connection`] is given bytes that arrived and a buffer to write
into, and it answers with what it wants sent, what it has to give its
caller, and what it is waiting for. The generator, the private key, the
rule that admits a host key and the moment are all parameters.

What it does, in order: the identification string, the negotiation and
the key exchange, `publickey` authentication, one `session` channel with
`exec` or `shell`, and a re-exchange whenever either side asks for one.
[`client::Event`] is what the caller acts on — bytes to send, bytes to
read, the command started, data on either stream, the exit status, the
end.

Its buffers are the caller's and their minimum is the packet size RFC
4253, section 6.1, makes mandatory, which is what one connection costs.

## What is not here

The socket under the client and the program around it, which are the rest
of step S8 and need the network on the machine, and the handshake against
an OpenSSH that measures this client against an implementation this
project did not write.
