# server-console

What the console driver does: put bytes on the serial line, and keep the
bytes that arrive on it until a client asks for them.

The controller is `driver-uart16550`, which reaches its registers through
the `Registers` trait; the binary supplies an implementation over the port
system calls, and the tests supply the recording one that crate already has.
Nothing here makes a system call, so all of it runs on the host.

The driver runs two threads, and this crate is what the two of them share
knowledge of rather than state. One thread waits on the endpoint and owns
everything in here; the other waits on the interrupt, reads the byte the
controller has, and sends it to the first over the same endpoint. Nothing is
held by both, so nothing needs a lock.
