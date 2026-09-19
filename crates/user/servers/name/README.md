# server-name

The registry behind the name server: up to sixty-four names, the endpoint
each one stands for, and the badge of the client that put it there.

Nothing here makes a system call. The server binary receives a message,
decodes it with `user-proto`, asks this crate, and sends the answer back;
what is worth testing is in here and runs on the host.

What a client registers is not what the registry stores. The kernel copies
rights and badge onto the handle it installs in the receiver, so the handle
a server sends carries the `RECV` and `BADGE` of the server's own endpoint;
`Registry::accept` stores a handle narrowed to `SEND | TRANSFER` instead and
gives the sent one up, so a lookup hands out nothing above `SEND`. The
narrowing itself is a system call, which is why `accept` takes a `Handles`
the server binary implements on its gate.

Ownership is by badge, which is the only thing about a sender the kernel
guarantees. A name may be replaced by the client that registered it and by
nobody else, and everything a client registered goes when the root task
reports that the client is gone. Every entry the registry stops holding —
replaced, forgotten, or taken out with a dead client — gives its handle up,
because the registry holds the only copy the server has.
