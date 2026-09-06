# net-udp

UDP as RFC 768 defines it, with the checksum rules the two address
families do not share, and a fixed table of sockets above it.

A socket carries an `IpAddr` and belongs to no family. What differs
between the two is the checksum, and the difference is not cosmetic. Over
IPv4 the field is optional: a zero means the sender computed none and the
datagram is accepted as it is. Over IPv6 it is mandatory in both
directions, because there is no header checksum underneath it to catch a
corrupted address (RFC 8200, section 8.1), so a zero is a reason to
discard. A datagram whose sum comes out zero is sent as all ones in both
families, which is what RFC 768 asks for and what keeps the omitted form
distinguishable from a computed one.

The length field is what a reader believes. Bytes behind it are padding
the layer below left standing, and are cut away; a length that reaches
past the bytes there are is an error. The checksum covers exactly what
the length field claims, so the two agree.

Nothing is sent here. This crate writes a datagram into the caller's
buffer and says which addresses it was written for; which of the two
senders carries it out is the decision of the facade above, which is what
lets one socket serve both families.

A receive ring lies in memory the caller supplied and holds datagrams as
self-describing records: each carries the addresses it arrived between,
the port it came from, and its payload. A record is never cut in two at
the end of the buffer — one that no longer fits behind the last begins at
the front instead — so a payload is always one slice. A full ring drops
the newest datagram and counts it, because the datagrams already in it
are the older ones and a client that is behind is better served in order.

An ephemeral port is drawn from the dynamic range of RFC 6335, which is
exactly sixteen thousand three hundred and eighty-four ports wide: the
low fourteen bits of two random bytes name one of them, without the bias
a remainder over a range that is not a power of two would introduce
(D-51). A port already taken is answered by drawing again rather than by
walking to the next one, as RFC 6056, section 3.3.1 asks — a port beside
a taken one is a port an observer who saw the first can guess — and the
number of draws is bounded.
