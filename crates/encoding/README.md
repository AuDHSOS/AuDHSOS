# audhsos-encoding

The three text encodings the project reads and writes, each strict on the
way in and canonical on the way out. Every function writes into a buffer
the caller owns and answers how many bytes it wrote; nothing here
allocates, and nothing here panics.

Strict means that exactly one text stands for a given sequence of bytes,
and that everything else is an error. Base64 rejects whitespace, a
missing or excessive pad, a pad anywhere but at the end, and a final
quantum whose unused bits are not zero — the last of which is the
difference between a decoder that round-trips and one that lets two texts
mean the same thing. Hex writes lower case and reads either case, because
a hex dump is read by people and written by programs.

PEM follows the strict form of RFC 7468: the label on the end line must
be the one on the begin line, every body line but the last is exactly 64
characters, and nothing may follow the end line. Explanatory text *before*
the begin line is allowed, as the RFC permits, and is not returned — that
is how a certificate file with a human-readable preamble is read without
also accepting a file with something appended to it.

Decoding a block yields two borrows: the label points into the input, the
bytes point into the buffer the caller supplied. The crate never sees a
`Vec`, so the caller decides where a certificate lands.
