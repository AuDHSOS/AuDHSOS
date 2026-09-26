# user-proto

The messages the servers of `AuDHSOS` speak, each one a type with `encode`
and `decode` and no system call in it. `label::Protocol::ALL` lists the
eight protocols: name, console, memory, parent, display, input, file, and
socket.

A label carries the version in its high sixteen bits, the protocol in the
next sixteen, and the message in the low sixteen, so a server can tell a
message of a version it does not speak from one it does before it looks at
a single word.

`handles::Carried` is the one part that is not a message: the capabilities
the kernel installed for a received message, read out before any other
call of the program overwrites the buffer, so that a server can give up
every handle its request did not take (D-187).

Every reply begins with a status word — zero, or the code of the error the
server answers with — so that a client reads the outcome of a request from
the same place whatever the request was. The parent protocol has no reply:
its one message is what a child says on its way out, and there is nobody
left to answer.
