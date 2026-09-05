# audhsos-elf

The ELF64 parser shared by the UEFI loader, which reads the kernel image,
and the userland program loader, which reads every other program. It
validates before it reports: an `Image` names only segments that lie inside
the file, inside the caller's address bounds, do not overlap, and are never
both writable and executable. Parsing borrows the input and allocates
nothing.
