# audhsos-symbols

An address to a function, a file, and a line. The crate reads the symbol
table of an ELF file for the function and the DWARF line program for the
file and the line, and it reads nothing else: no inline frames, no
call-frame information, and therefore no stack unwinding.

Line programs of DWARF version 4 and version 5 are understood. Everything
borrows from the bytes it was handed, so the crate allocates nothing and
runs where the kernel runs, although only the build automation uses it: it
turns the address a kernel panic prints over the serial line into a place
in the source.
