# net-stack

The facade. One interface, one `poll`, one `poll_at`, and under them
every other crate of the track: frames and ARP, both internet layers with
their reassembly and their two `ICMP`s, UDP sockets and TCP connections,
address configuration by DHCP and by router advertisement, and the
resolver. A server process is a loop around `poll` and nothing else.

`poll(now, rx, tx)` takes in at most one frame and hands out at most one.
What a received frame makes — an answer to it, and the datagram that was
waiting for the neighbor it just taught the cache about, cut into as many
pieces as the MTU needs — goes into a queue in memory the caller supplied,
and comes out one frame per call in the order it went in. The layers are
asked for new work only when that queue is empty, which is what makes a
transmit buffer of one frame enough: nothing is written while there is a
backlog, so nothing is lost and nothing overtakes anything.

Two limits are the caller's and the rest are this host's. How many sockets
and how many connections is what an application sizes; how many routes,
how many neighbors, how many reassembly buffers and how many prefixes are
properties of a host with one interface, whatever runs on it. So the type
carries two numbers and the crate carries the others as constants that say
what they are.

A handle is an index and the generation of the slot it names. A bare index
is a handle that comes back to life — the socket is closed, the slot is
used again, and a caller holding the old number is reading somebody else's
socket with no error anywhere — and the answer is the kernel's: the slot
counts its uses, the handle carries the count it was made under, and the
two have to agree.

Two things do not go through either of the two senders, and both for the
same kind of reason. Neighbor Discovery needs a hop limit of 255 —
RFC 4861, section 7.1 requires it, and it is the whole of what makes the
protocol link-local — and it needs no route and no neighbor resolved,
which is fortunate, since resolving one is what it is for. The address
configuration client needs to send before this host has an address, and a
host with no address has no route either. Both write their own header and
go straight onto the link.

An address this host forms from a router advertisement is tentative until
duplicate address detection has finished with it (RFC 4862, section 5.4).
The route to the prefix goes in at once, because a prefix is about the
link and not about this station; the address goes in when nobody has
claimed it, and if somebody has, this host is left without one rather than
guessing another.

Address selection is RFC 6724. Its policy table is written over IPv6
prefixes with IPv4 standing in as the mapped range, and this system has no
mapped addresses at all (D-69), so that one row is read as the row of the
IPv4 family rather than by forming an address the rest of the stack would
refuse. Of the rules, the ones a host with one interface, no deprecated
addresses, no Mobile IPv6, no privacy extensions and no tunnels can decide
are here; the rest decide nothing here and are named in `select` so that
their absence is not mistaken for an oversight.

Connecting to a list of addresses and connecting to a name are two
operations and not one. `connect_to_any` tries a list in the order it is
given, moving to the next when a candidate times out or is reset;
`connect_to_name` resolves, orders what came back by RFC 6724, and hands
that list to the same loop. They are not raced: Happy Eyeballs (RFC 8305)
is a policy about how much of somebody's network to spend on a guess, and
it belongs above a stack rather than in one. What its absence costs is one
timeout on a path the routing table does not know is broken.
