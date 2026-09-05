# net-ip

IPv4 as RFC 791 defines it, ICMP as RFC 792 does, and the routing table
that both address families share.

A datagram is a borrowed view over the bytes it arrived in. Options are
located and skipped, never interpreted: this host originates none and
needs none to read a datagram, and a parser that understood them would be
reading a field an attacker writes.

The routing table holds IPv4 and IPv6 routes together, because a host has
one set of destinations and not two. A route of one family never matches
a destination of the other, and the match is the longest prefix, so a
host route beats a subnet route and a subnet route beats the default.

Errors are generated under the restrictions of RFC 1122, section 3.2.2,
which take precedence over every other reason to send one: never in
answer to an error, to a broadcast or multicast destination, to a
non-initial fragment, or to a datagram whose source names no single host.
A token bucket bounds what is left.
