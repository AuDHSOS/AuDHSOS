# kernel-hal-api

The traits through which the kernel logic reaches hardware: page-table
memory, TLB control, frame supply, the debug console, the test exit device,
the timer, the interrupt controller, port I/O (feature `port-io`), and the
boot platform. The feature `test-doubles` adds in-memory implementations
that record every call, so that every algorithm written against these
traits runs on the host.
