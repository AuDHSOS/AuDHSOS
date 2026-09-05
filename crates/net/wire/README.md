# net-wire

The primitives every layer of the network stack shares: the addresses, the
two protocol tables, a cursor that reads and writes big-endian integers
without ever leaving its buffer, and the internet checksum of RFC 1071.

Nothing here knows a protocol. A header is parsed one layer up, out of a
byte slice, with this crate's cursor; what the cursor gives back is a
borrow into that slice, so a packet is read where it landed and copied
nowhere. Every fallible operation returns a `Result`, and a failed read or
write leaves the cursor exactly where it was — a parser that has run out
of bytes may report and stop, and does not have to unwind a half-consumed
position.

The addresses are values, not strings. `MacAddr`, `Ipv4Addr`, and
`Ipv4Cidr` parse from their canonical text and write it back, and the
parse is strict: `010.0.0.1` is refused rather than read as octal, and a
prefix length of 33 is an error rather than a mask of all ones. What comes
out of `Display` is what goes into `parse`, and there is exactly one text
per address.

The checksum accumulator carries the end-around carry of RFC 1071 on every
16-bit word instead of deferring it, so the sum provably stays inside
sixteen bits and no addition in this crate can overflow. It also carries a
pending byte across calls, which is what lets a caller feed it a
pseudo-header, then a header, then a payload, without the odd length of
any one of them shifting the bytes of the next.
