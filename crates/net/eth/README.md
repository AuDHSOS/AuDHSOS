# net-eth

The link layer: Ethernet II frames, the address resolution protocol of
RFC 826, and the neighbor cache both address families share.

A frame is a borrowed view over the bytes it arrived in. Reception is a
filter and not a parse failure — a frame shorter than its header, one
addressed to another station, and one carrying a type no layer here reads
are all dropped, quietly, because a link carries other people's traffic
and a stack that reports each piece of it as an error is a stack that
reports nothing useful.

The neighbor cache is one cache for IPv4 and IPv6, keyed by `IpAddr`,
with the five states of RFC 4861. ARP fills its IPv4 half; Neighbor
Discovery, which is `ICMPv6` and therefore lives a layer up, fills the
other. This crate owns the storage, the states, and the timers, and
neither of the two protocols: it says *ask again for this address now*
and the caller writes whichever packet that family takes.

Nothing here reads a clock. Time arrives as an `Instant`, `poll_at` says
when the cache next has work, and a test drives a sixty-second schedule
in microseconds of wall clock.
