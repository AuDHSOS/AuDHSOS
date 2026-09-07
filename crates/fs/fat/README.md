# fs-fat

The structural logic of a FAT32 file system: the boot parameter block,
the cluster chains, the directories in 8.3 form, and reading and writing
a file. It touches no device. Everything it does passes through a
[`BlockDevice`], which hands over one sector at a time, and what that
sector is made of — a disk, a partition inside an image, a byte vector in
a test — is the caller's business.

FAT32 only. A FAT12 or a FAT16 volume is refused at the boot sector
rather than read badly, because the two use the root directory and the
table width differently and a reader that guessed between them would be
three file systems in one. No long file names either (D-09): a name is
the eleven bytes of the 8.3 form, and a name that does not fit them is
refused at the call rather than truncated.

Nothing here allocates. The largest thing on the stack is one sector.

## What a caller does with it

`FileSystem::format` writes a fresh volume onto a device and
`FileSystem::mount` reads the one that is there. Both answer a
[`Geometry`], which is the boot parameter block as numbers: where the
tables are, where the data starts, how many clusters there are. From a
mounted file system a caller opens the root, walks it with `entries`,
finds a name with `find`, makes a file with `create` or a directory with
`create_dir`, and reads and writes bytes at an offset.

A file is [`File`], which is a first cluster, a size, the directory slot
it is described by, and a cursor. The cursor is why writing a large file
is not quadratic: a chain has no back pointer, so a write that had to
find its place from the first cluster every time would walk the whole
file again for every cluster it added. A write that continues where the
last one stopped walks nothing.

## What it refuses

A cluster chain is walked with a step count bounded by the number of
clusters in the volume, so a chain that points back into itself is an
error and not a hang. Four things end a walk: the end-of-chain marker,
which is the ordinary end; a free cluster, which means the chain and the
table disagree about what is in use; the bad-cluster marker; and a number
outside the table. Only the first is success.

A directory entry is skipped when it is deleted, when it is the volume
label, and when it belongs to a long file name. It is refused when its
eleven bytes are not a name this crate would write — a lower-case letter
is the case that matters, because the short form has no room for the flag
that would say what the case meant.

The time in a directory entry goes through `audhsos-time`. FAT counts
years from 1980 and seconds in twos, so what round-trips is an even
second between 1980 and 2107; anything else is refused rather than
rounded silently. The moment an entry carries is the one the file was
made with, and a write does not move it: the one caller this crate has
writes an image that has to come out the same bytes twice, and a caller
that wants a modification time writes the entry itself.
