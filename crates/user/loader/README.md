# user-loader

What the root task has to read before it can start anything: the ustar
archive in the boot image, and the segments of a user program's ELF.

Both are parsers over borrowed bytes with no system call in them, so both
run on the host and under the fuzzer. What the root task does with what
they say — allocate, map, copy, unmap, map again — is the wiring in the
program that uses this crate.

The reader is strict where a loader has to be. A name that is absolute or
that walks upwards through `..` is refused, because an archive is untrusted
input and a path out of it names a file the root task has no business
opening; a header whose checksum does not match is refused; and the archive
ends where its end marker says, whatever follows it.
