# user-proto

The messages the servers of `AuDHSOS` speak, each one a type with `encode`
and `decode` and no system call in it: the name protocol, the console
protocol, and the memory protocol.

A label carries the version in its high sixteen bits, the protocol in the
next sixteen, and the message in the low sixteen, so a server can tell a
message of a version it does not speak from one it does before it looks at
a single word.

Every reply begins with a status word — zero, or the code of the error the
server answers with — so that a client reads the outcome of a request from
the same place whatever the request was.
