# net-dhcp

The client half of RFC 2131, over IPv4 and over nothing else. IPv6
configures itself from a router advertisement, which is `ICMPv6` and
therefore lives in `net-ipv6`; there is no `DHCPv6` (D-69).

The four messages of the exchange are here — discover, offer, request,
acknowledge — with the negative acknowledgment that sends a client back to
the start, and the lease timers behind them: renewal at T1 by unicast to
the server that granted the lease, rebinding at T2 by broadcast to whoever
will listen, and an expiry that takes the address away again. What is not
here is the rest of the message types. There is no decline, no release, no
inform, and no init-reboot: each is an optimisation or a courtesy, and
none of them is on the path from a host with no address to a host with
one.

The client sends nothing. It writes a complete UDP datagram into the
caller's buffer and names the two addresses it was written for, which is
what D-84 has a transport do; the datagram goes from port 68 to port 67,
which RFC 2131 fixes, so the only randomness this crate draws is the
transaction id and the jitter on the backoff (D-51).

The BROADCAST flag is set until the address stands. A client that has no
address cannot accept an IP datagram addressed to the one it has just been
offered, and RFC 2131, section 4.1 has exactly this flag for exactly that
deadlock: with it set, the server answers to 255.255.255.255 and the
wildcard socket on port 68 takes the answer without any layer below
knowing about an address this host does not yet have. From the bound state
on — renewal and rebinding — the flag is clear, because by then the
address is configured and a unicast reply arrives.

The backoff is the one RFC 2131 asks for: four seconds, doubled to a
ceiling of sixty-four, each delay moved by a uniform value between minus
one and plus one second. In the renewing and rebinding states it is the
rule of section 4.4.5 instead — half of what is left until the next
deadline, and never less than sixty seconds.

An address a host cannot hold is refused before a lease is made of it: the
unspecified address, the limited broadcast, a multicast group, and a
loopback address are none of them one host on one link. What is not
refused is the network address or the directed broadcast of the prefix
that comes with the lease, because RFC 3021 gives a point-to-point link a
/31 on which both of those are hosts and a /32 lease is one address that
is all three at once.

An option is read strictly. The end marker is required, padding is
skipped, an option this client has no use for is stepped over, and one
whose length reaches past the block is an error rather than a short read.
A subnet mask that is not a prefix — bits that do not run together — is
refused, because it cannot be a network and a route from it would be a
guess.
