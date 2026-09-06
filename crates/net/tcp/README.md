# net-tcp

TCP as RFC 9293 defines it: the eleven states, the sequence arithmetic
they are decided by, the retransmission timer of RFC 6298, and the Reno
congestion control of RFC 5681.

The whole of it is a function of state, input, and `now`. Nothing here
blocks, allocates, sleeps, or reads a clock: a caller hands a connection
the segment that arrived and the instant it arrived at, and asks
`poll_at` when it next has work. That is what lets a sixty-second
retransmission backoff be exercised in microseconds of wall clock, and it
is what lets two instances of this machine be connected back to back over
a network double that delays, duplicates, reorders, and drops — which is
the test that makes the rest of it trustworthy.

Passive open is here although the first client needs only active open,
for that reason and no other. What is not here is a backlog: one
connection may stand in `LISTEN`, and the first `SYN` that matches makes
it that connection. A server that accepts several at once is a queue
above this crate, and nothing in this project has asked for one.

The send and receive windows lie in ring buffers the caller supplied. A
segment that arrives out of order is written straight to its place in the
receive ring — its sequence number says where that is — and a short list
of ranges remembers which parts have arrived; when the gap in front of
them closes, the ranges grow together and the bytes are readable without
having been copied twice. The list is of fixed size, so a connection that
is missing more pieces than it has room for drops the newest and lets the
peer send it again.

The options are one: the maximum segment size. No window scaling, no
selective acknowledgment, no timestamps, no keepalive (D-50). What that
costs is throughput over a path with a large bandwidth-delay product,
which QEMU's virtual link does not have. The boundary is written down so
that the omission is not mistaken for a defect.

Resets are handled under the rules of RFC 5961: a reset is believed only
when its sequence number is the one expected, and a reset or a `SYN`
inside the window but not at its edge draws a challenge acknowledgment
rather than a teardown. A blind attacker who guesses a window therefore
has to guess a sequence number as well.
