# net-ipv6

IPv6 as RFC 8200 defines it, `ICMPv6` as RFC 4443 does, Neighbor
Discovery of RFC 4861, the stateless address configuration of RFC 4862
with the recursive DNS server option of RFC 8106, and the path MTU
discovery of RFC 8201.

A packet is a borrowed view over the bytes it arrived in. Its extension
headers are stepped over to reach the upper-layer header, and the walk is
bounded in the number of headers and in the bytes they take: a chain
without end is this format's compression pointer loop, and the answer to
it is a drop.

There is no header checksum and therefore nothing to verify. What catches
a corrupted header is the upper-layer checksum, which RFC 8200,
section 8.1 makes mandatory over IPv6 where IPv4 left it optional for
UDP, and which is summed over a pseudo-header that names both addresses.
`ICMPv6` is summed over that pseudo-header as well, which is the one
difference from `ICMPv4` a checksum routine has to know.

A router never fragments an IPv6 packet, so the source does it or it does
not happen. That is why path MTU discovery is not optional here, and why
the estimate has a floor of 1280 bytes that no message can push it below.

What is shared with IPv4 is not repeated: the routing table, the
reassembly buffers, and the fragmentation arithmetic are `net-ip`'s, and
the neighbor cache is `net-eth`'s, which Neighbor Discovery writes into
rather than keeping a second one (D-69). There is no `DHCPv6`.
