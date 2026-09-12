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

## What is not here

The cipher, which is step S3 and makes the block size something other
than eight; the MAC, which
`chacha20-poly1305@openssh.com` makes unnecessary; and everything above
the packet: the negotiation, the key exchange, the authentication and
the channels.
