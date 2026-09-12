# audhsos-ssh

The SSH-2 client of [document 14](../../../docs/14-secure-shell-as-a-client.md),
sans-I/O: it is given bytes that arrived and a buffer to write into, and
it never reads a socket, allocates, or asks what time it is. What exists
today is step S1 of track S, the two layers everything else is written
in.

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

## What is not here

Everything above the transport: the host key blobs and the signature
over the exchange hash (S4), the authentication (S5), the channels (S6),
and the re-exchange (S7).
