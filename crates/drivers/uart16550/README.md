# driver-uart16550

The register logic of the 16550 serial controller: the initialization
sequence, the polled transmit path with a bounded number of polls, the
receive path, and the interrupt registers. The crate reaches the hardware
only through the `Registers` trait, so the same logic serves the kernel
debug console over port I/O and the userland console driver over the port
system call. The feature `test-doubles` adds a recording implementation of
the trait.
