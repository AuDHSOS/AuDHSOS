# net-dns

The message format of RFC 1035 and a stub resolver over it. A name is
asked for in `A` and in `AAAA`, and the answers of both come back
together: which family a host reaches a name over is the question of the
layer that opens the connection, and answering it here would answer it
twice.

Names are read with compression and written without it. A compression
pointer must point strictly backwards of the pointer itself, which
refuses a pointer to itself or forwards before the reader follows the
pointer. The walk reads labels forwards, so a pointer behind a label may
target that label; the jump count and the bound of 255 bytes of RFC 1035,
section 2.3.4 on the name being assembled end such a cycle.

A decoded name is a value of its own and not a borrow into the message.
It cannot be a borrow: a compressed name is not contiguous in the bytes it
arrived in. And it must not be one, because the resolver has to hold the
name it is asking across the datagrams it asks in.

`A` and `AAAA` records are decoded, `CNAME` is decoded to the name it
points at, and every other type is carried as the bytes of its body and
ignored. A record of a class other than `IN` is carried the same way: it
is not an internet address, whatever its type says.

The resolver is a state machine and moves no bytes. It writes a complete
UDP datagram into the caller's buffer and names the two addresses it was
written for, as D-84 has a transport do, and it takes a response as the
payload with the address and port it arrived from — which is exactly what
a receive record of `net-udp` carries. The transaction id comes from
`Rng`; the source port is the port of the socket the caller bound, which
`net-udp` drew from `Rng` in the dynamic range (D-51).

A response is believed only when the source address is one of the servers
this resolver was given, the source port is 53, the transaction id is the
one that went out, and the question section is the question that was
asked: the name, the type, and class `IN`. It is also read only up to 512
bytes: this resolver announces no buffer of its own, so RFC 1035, section
4.2.1 is what a server may send it, and a datagram past that is not an
answer to anything it asked. Three of the four are what an off-path
attacker has to guess, and the fourth is what stops one answer from being
taken for another.

The two questions are in the air at once, each with its own id, its own
attempt counter and its own place in the server rotation, under one
deadline. A resolution ends when neither question is open; a family that never
answers costs nothing but its own attempts, where asking one after the
other would spend the whole deadline on the first.

A `CNAME` chain is followed inside the answer it arrived in, and where it
ends at a name the answer carries no address for, the resolver asks that
name. One budget of eight links governs the chain however many messages it
is spread over, and a record the chain has already stepped through is a
loop and ends it.
