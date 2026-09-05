# kernel-acpi

The ACPI tables the kernel needs to find its interrupt controllers, parsed
in safe Rust. The firmware leaves a root pointer behind; the pointer names
a root table; the root table names every other table; one of them, the
multiple APIC description table, says where the local APIC and the I/O
APICs are and how the ISA interrupt lines are wired to them.

Every parser here takes bytes and returns a value or an error. It borrows
the input, allocates nothing, and validates before it reports: signature,
length, and checksum first, then the fields. The adapter that copies the
bytes out of physical memory lives in `kernel-hal-x86_64`; this crate never
sees an address it dereferences.
