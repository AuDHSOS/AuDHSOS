# server-name

The registry behind the name server: up to sixty-four names, the endpoint
each one stands for, and the badge of the client that put it there.

Nothing here makes a system call. The server binary receives a message,
decodes it with `user-proto`, asks this crate, and sends the answer back;
what is worth testing is in here and runs on the host.

Ownership is by badge, which is the only thing about a sender the kernel
guarantees. A name may be replaced by the client that registered it and by
nobody else, and everything a client registered goes when the root task
reports that the client is gone.
