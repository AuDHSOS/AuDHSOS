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

The addresses are values, not strings, and both families are here.
`MacAddr`, `Ipv4Addr`, `Ipv4Cidr`, `Ipv6Addr`, and `Ipv6Cidr` parse from
their canonical text and write it back, and the parse is strict:
`010.0.0.1` is refused rather than read as octal, `2001:0db8::1` is
refused because RFC 5952 suppresses the leading zero, and a prefix length
of 33 is an error rather than a mask of all ones. What comes out of
`Display` is what goes into `parse`, and there is exactly one text per
address.

`IpAddr` and `IpCidr` are what the layers above carry, so that a socket, a
route, and a neighbor are written once for both families. There is no
IPv4-mapped form: the two families are distinct values, and an operation
over one address of each is an error rather than a conversion.

The checksum accumulator carries the end-around carry of RFC 1071 on every
16-bit word instead of deferring it, so the sum provably stays inside
sixteen bits and no addition in this crate can overflow. It also carries a
pending byte across calls, which is what lets a caller feed it a
pseudo-header, then a header, then a payload, without the odd length of
any one of them shifting the bytes of the next. Both pseudo-headers are
here — the IPv4 one of RFC 793 and the IPv6 one of RFC 8200, section 8.1,
which is wider, carries a 32-bit length, and names the upper-layer
protocol rather than the packet's next-header field.
