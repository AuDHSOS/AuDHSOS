# kernel-hal-x86_64

The `x86_64` side of the kernel: the privileged instructions, the
descriptor tables the processor needs before it can take a trap, the trap
handlers and the handler of every device vector, the ACPI tables read
through the window, the local APIC and the I/O APICs over their register
windows, the legacy interrupt controllers moved out of the way and masked,
the timer measured against the interval timer, the boot information the
loader left behind, the window every physical frame is reachable through,
the lookaside buffer and the page-table root, the debug console over the
serial port, and the exit device the test runner reads. Every `unsafe`
block here does one thing and says which precondition makes it sound;
everything that can be decided without a machine lives in
`kernel-x86-tables`, `kernel-acpi`, or `kernel-core` instead.
