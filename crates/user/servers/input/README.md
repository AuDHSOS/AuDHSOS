# server-input

What the input server decides, apart from the system calls it makes: who is
subscribed, which of the two decoders a byte from the controller belongs to,
what is appended to whose ring, and which subscriber is dropped.

A client is known by the badge of the capability its messages arrive
through, so a ring belongs to whoever subscribed and a request that carries
no badge names nobody and is refused.

Nothing here maps memory, makes a system call, or touches a port. The
process around it hands in the bytes of each subscriber's ring and a way to
wake it, and this crate says what goes into which ring and who has gone
away. That is why it is tested on the host, over byte vectors that stand in
for the rings and a recording double for the wake-up.
