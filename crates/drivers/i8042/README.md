# driver-i8042

The i8042 controller of a PC: the initialization sequence, the one output
buffer that carries the bytes of two devices, and the decoders that turn
those bytes into key and pointer events.

The crate reaches the hardware only through the `Ports` trait, so the same
logic serves the input server over the port system calls and a test over a
scripted register file. It depends on nothing, which is why the event types
live here and `user-proto` re-exports them.

The controller has one output buffer and two lines. Which device a byte
came from is the `AUX` bit of the status register, read together with the
byte, so one thread drains the buffer and hands each byte to the decoder
the bit names. The feature `test-doubles` adds the scripted implementation
of `Ports` the tests use.
