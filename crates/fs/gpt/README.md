# fs-gpt

The GUID partition table as structures: the protective record in the
first block, the two headers with the checksums that say they were not
torn, and the array of entries that says where the partitions are. It
touches no device. Everything it does passes through the
[`BlockDevice`](fs_fat::BlockDevice) of `fs-fat`, which hands over one
block at a time, so the image writer and a file system server read the
same code.

The layout is UEFI 2.11, sections 5.2.3 and 5.3, and every offset here
names the table it came from.

## What a caller does with it

`read` answers the header of the table that is there. It checks the
protective record, then the primary header in block 1, and where that is
torn it reads the backup in the last block, which is the recovery the
specification asks for. From the header, `find` answers the first
partition of a type — `ESP_TYPE_GUID` being the one this system boots
from — and `next_entry` walks all of them behind a `Cursor` that keeps
the block it is inside, so the walk costs O(A) block reads in the blocks
of the array rather than one read per entry.

`write` lays down a fresh table: the protective record first, then both
arrays, then the backup header, then the primary. The order is what a
write cut short leaves behind. Without the record a reader takes the
device for one partitioned the legacy way and reads no table at all; and
a backup that is already down can be read, while a primary that names an
array nobody wrote cannot.

## What it refuses

A header whose signature, revision, size, or checksum is not what the
format says, and one that does not lie in the block it names as its own.
An entry array whose checksum does not cover its bytes. A device whose
first block is a legacy partition table, because a table found behind
one is what an older tool left standing. An entry that lies outside the
range the header calls usable.

Entry sizes: the format allows 128 times any power of two, and this
crate reads those that also divide a block, so that no entry straddles
two of them. In practice every writer uses 128.

Nothing here allocates. The largest thing on the stack is one block, and
the cursor of a walk carries one more.
