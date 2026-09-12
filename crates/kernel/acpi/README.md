# kernel-acpi

The ACPI tables the kernel needs to find its interrupt controllers, parsed
in safe Rust. The firmware leaves a root pointer behind; the pointer names
a root table; the root table names every other table; one of them, the
multiple APIC description table, says where the local APIC and the I/O
APICs are and how the ISA interrupt lines are wired to them.

The document is the *ACPI Specification* 6.6, kept in
[`docs/uefi/`](../../../docs/uefi/README.md): section 5.2.5.3 for the root
pointer, 5.2.6 for the table header, 5.2.7 and 5.2.8 for the RSDT and the
XSDT, 5.2.12 and its subsections for the MADT. The `MCFG` table is the one
exception, the *PCI Firmware Specification* 3.3, section 4.1.2, which
[`docs/pcisig/`](../../../docs/pcisig/README.md) records as unobtainable;
only its table header comes from ACPI 6.6.

Every parser here takes bytes and returns a value or an error. It borrows
the input, allocates nothing, and validates before it reports: signature,
length, and checksum first, then the fields. The adapter that copies the
bytes out of physical memory lives in `kernel-hal-x86_64`; this crate never
sees an address it dereferences.
