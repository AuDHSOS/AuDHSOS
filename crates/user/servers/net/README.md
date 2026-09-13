# server-net

The network server as logic: the socket table of every client, the two
rings every socket's bytes travel through, and the loop around
`net-stack`. No system call is in it — the process around it is a binary
of `user-programs`, as every other server is.

## What a round is

A frame the driver took goes into [`Server::poll`], which hands it to
`Stack::poll` and answers the frame the stack wants sent, if any. The
caller loops until nothing comes back and then asks [`Server::poll_at`]
for the instant the stack next has work at.

Every round moves bytes between the connections and the rings: what a
connection holds goes into the inbound ring as far as the ring has room,
and what the client put into the outbound ring goes into the connection as
far as its window allows. A client that stops reading fills its ring, the
server stops taking bytes out of the connection, and the window stops
advancing — which is the back pressure TCP already has, and why nothing
here grows.

## What a client gets

A socket is a number this server chose, and the number carries the
generation of the slot it names, so a number of a socket that was closed
is refused rather than answered for the socket that took the slot (D-116).
Which client a number belongs to is the badge of the endpoint the message
came through, as every other server of this system decides it.

The rings of a socket are one memory object of two pages, made by the
process around this crate before the first client arrives: a pool of as
many as [`MAX_SOCKETS`] says, handed out with a socket and taken back with
it. Nothing is allocated while a client is being served.

## What is answered at once

Every call. A connection that is not open yet, an accept with nothing to
take, and a resolution still running each answer `WouldBlock`, and the
client asks again (D-142). The alternative — holding the reply capability
until the answer is there — would stall every other client behind the one
that is waiting, the server holding one reply at a time.
