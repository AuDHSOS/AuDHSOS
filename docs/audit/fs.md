# server-fs, fs-fat, fs-gpt audit findings

Repository: AuDHSOS/AuDHSOS. Audit of server-fs, fs-fat, fs-gpt at main bc47df5.
Each `## ` section is one issue, filed 2026-09-19 with the number in its heading. `Title:` is the issue title, `Labels:` the labels, everything after `Body:` is the issue body verbatim.

---

## F01 — issue #211
Title: server-fs: a handle kept open across a remove patches the directory slot another name now holds
Labels: bug, part::fs
Body:
`remove` at `crates/user/servers/fs/src/serve.rs:371-383` deletes the entry through `fs.remove` and never consults `Clients`, so every `Opened::File` that names the entry (`crates/user/servers/fs/src/open.rs:53-62`) keeps its `File` with the `Location` of the deleted slot (`crates/fs/fat/src/fs.rs:66-67`). `insert` at `crates/fs/fat/src/fs.rs:661-667` gives the next `create` the first slot whose first byte is `0` or `DELETED`, which is that slot. `reserve` at `crates/fs/fat/src/fs.rs:611-617` and `write` at `crates/fs/fat/src/fs.rs:532-535` call `write_back` at `crates/fs/fat/src/fs.rs:541-549`, which patches first cluster and size into whatever entry stands at the stale location. `seek` at `crates/fs/fat/src/fs.rs:584-603` answers the cursor cluster for index 0 without a table read, so a read through the stale handle reads the cluster whoever now owns it.

Sequence: client A sends `Create A.TXT` (slot s), `Write` 512 bytes (cluster c), and keeps the handle. Client B sends `Remove A.TXT` (`free_chain` at `crates/fs/fat/src/fs.rs:444` frees c and sets `next_free` to c at `crates/fs/fat/src/fs.rs:240`; slot s is marked deleted), then `Create B.TXT` (slot s again), then `Write` its data (`find_free` at `crates/fs/fat/src/table.rs:164-193` hands out c). Client A sends `Read` at offset 0 and receives B's bytes from c. Client A sends `Write` at offset 0: `write_back` patches B.TXT's entry with A's first cluster and size, so B.TXT names A's chain and B's chain is orphaned. The invariant at `crates/user/servers/fs/src/serve.rs:17-18`, that no client reaches what another opened, does not hold.

Fix: `remove` in `serve.rs` refuses with `Error::Busy` while any table in `Clients` holds an `Opened::File` with the same volume, parent and name; dropping those handles on remove instead would answer the holder `InvalidHandle` at its next message without the holder having done anything.

---

## F02 — issue #212
Title: fs-fat: a directory whose cluster chain loops keeps the entry walk running forever
Labels: bug, part::fs
Body:
`next_entry` at `crates/fs/fat/src/fs.rs:268-298` loops `while !cursor.done`, and `step` at `crates/fs/fat/src/fs.rs:695-706` follows the chain through `table::next` (`crates/fs/fat/src/table.rs:112-131`) with no step count. `Entries` at `crates/fs/fat/src/fs.rs:45-52` holds no count. `insert` at `crates/fs/fat/src/fs.rs:670-673` has the bound that `next_entry` lacks. The README at `crates/fs/fat/README.md:38-40` claims every chain walk is bounded by the cluster count.

Trigger: a directory of two clusters A and B, the table entries A→B and B→A, every slot of both holding `0xE5` in its first byte. `decode` at `crates/fs/fat/src/dir.rs:140-142` answers `Slot::Free` for each, `next_entry` continues, `step` moves A→B→A without end. `find` at `crates/fs/fat/src/fs.rs:305-313` never returns; the server's `open` at `crates/user/servers/fs/src/serve.rs:173-176` never answers, and the server serves one request at a time, so every client waits from then on. The volume is untrusted data, so a crafted boot disk triggers this at the first `Open` under that directory.

Fix: `Entries` carries a cluster count that `step` increments at every cluster change and compares with `geometry.clusters`, answering `Error::ChainLoop` as `insert` does; bounding the slot count instead would need the chain length first, which is a second walk.

---

## F03 — issue #213
Title: server-fs: a disk with a legacy partition table or a damaged information sector is formatted
Labels: bug, part::fs
Body:
`whole` at `crates/user/servers/fs/src/volume.rs:73-81` formats the disk whenever `fs_fat::info` answers `Ok(None)`, and `info` at `crates/fs/fat/src/fs.rs:740-747` answers `None` when sector 1 lacks the two FSInfo signatures. `mount` reaches `whole` on `Error::NotProtective` (`crates/user/servers/fs/src/volume.rs:67`), which `crates/fs/gpt/src/error.rs:16-20` defines as a first block that is a legacy partition table. `is_partitioned` at `crates/user/servers/fs/src/volume.rs:33-38` answers `false` for the same disk, so `sort` at `crates/user/programs/src/bin/server_fs.rs:195-203` makes it the written disk. The invariant at `crates/user/servers/fs/src/volume.rs:17-18` and risk 5 at `docs/15-the-disk-on-the-machine.md:825` state that the server formats only where it found no volume.

Trigger 1: a disk whose sector 0 is a legacy MBR (signature `0x55 0xAA`, one record of type `0x0C`) and whose sector 1 is zero. `fs_gpt::read` answers `NotProtective`, `info` answers `None`, `format` at `crates/fs/fat/src/fs.rs:114-131` writes a boot sector over the MBR and empty tables over the first partition. Trigger 2: a FAT32 volume with no partition table whose sector 1 was zeroed. The information sector is a hint (Microsoft FAT32 File System Specification 1.03, "FAT32 FSInfo Sector Structure and Backup Boot Sector": the values are not reliable and a wrong signature means the fields are ignored); the boot sector says whether a volume is there ("Boot Sector and BPB"). The volume is formatted and its files are gone.

Fix: `whole` decides by `fs_fat::read_geometry`: `Ok` mounts, `Err(Error::Signature)` formats, every other refusal is answered with `refusal`; a legacy MBR then fails at `SectorSize` and is refused, not formatted. Keeping `info` as the test instead leaves both triggers.

---

## F04 — issue #214
Title: server-fs: two handles on one file write each other's size back
Labels: bug, part::fs
Body:
`open` at `crates/user/servers/fs/src/serve.rs:186-191` makes a fresh `File` for every `Open`, and `File` at `crates/fs/fat/src/fs.rs:61-72` carries its own `size`. `write` at `crates/fs/fat/src/fs.rs:532-535` raises `file.size` only when the write reaches past the copy's size, then `write_back` at `crates/fs/fat/src/fs.rs:541-549` patches that size into the entry unconditionally.

Trigger: one client sends `Open F.TXT` twice (handles h1 and h2, both `size` 0), `Write h1` at offset 0 with 1000 bytes (entry size 1000), then `Write h2` at offset 0 with 10 bytes: h2's copy has `size` 0, `end` 10 is larger, the entry size becomes 10. `read` at `crates/fs/fat/src/fs.rs:464-466` refuses offsets above 10, so bytes 10 to 1000 are unreachable. Two clients that open the same name get the same result.

Fix: `Clients` keeps one `File` per volume and location and every `Opened::File` refers to it, so all handles see one size and one cursor; re-reading the entry before every write in `fs-fat` instead costs a directory walk per write.

---

## F05 — issue #215
Title: fs-fat: the first cluster of an entry is not checked before its sectors are read
Labels: bug, part::fs
Body:
`decode` at `crates/fs/fat/src/dir.rs:154` takes `first_cluster` from the entry unchecked. `open` at `crates/fs/fat/src/fs.rs:321-333` and `open_dir` at `crates/fs/fat/src/fs.rs:341-348` pass it on, and `serve.rs` builds `Dir::at(entry.first_cluster)` itself at `crates/user/servers/fs/src/serve.rs:177-184`. `seek` at `crates/fs/fat/src/fs.rs:584-603` answers the cursor for index 0 without a table read, `next_entry` at `crates/fs/fat/src/fs.rs:275-279` reads the first cluster's slots before `step` consults the table, and `cluster_sector` at `crates/fs/fat/src/boot.rs:114-120` maps every cluster below 2 to the data start.

Trigger 1: a file entry with first cluster 1 and size 512: `Read` at offset 0 answers the first sector of cluster 2, the root directory. Trigger 2: a directory entry with first cluster 0: `ReadDir` lists the root directory's first cluster, `Create` in it writes into the root directory (`insert` at `crates/fs/fat/src/fs.rs:652-668`). The specification gives 0 as a directory's first cluster only in the `..` entry of a directory below the root (Microsoft FAT32 File System Specification 1.03, "FAT Directory Structure"), which `decode` skips at `crates/fs/fat/src/dir.rs:146`.

Fix: `next_entry` at `crates/fs/fat/src/fs.rs:286` answers `Error::Cluster` for an entry whose first cluster is not 0 and not held by the geometry, and for a directory entry whose first cluster is 0; checking in `open`, `open_dir` and the server's `Dir::at` instead is three places for one rule.

---

## F06 — issue #216
Title: fs-fat: remove frees the chain before it deletes the entry
Labels: bug, part::fs
Body:
`remove` at `crates/fs/fat/src/fs.rs:436-454` calls `free_chain` at line 444 and writes the deleted mark at lines 446-452 afterwards. `free_chain` at `crates/fs/fat/src/fs.rs:233-250` zeroes one table entry per step and returns an error midway on `FreeInChain`, `BadCluster` or `Cluster`, in which case the entry is never marked.

Trigger: power is cut between `crates/fs/fat/src/fs.rs:444` and `crates/fs/fat/src/fs.rs:452`. At the next boot the entry still names its first cluster c, which the table calls free. The next `create` and `write` take c (`next_free` is c after `crates/fs/fat/src/fs.rs:240`; `find_free` at `crates/fs/fat/src/table.rs:164-193`), so two entries name c. `Remove` of the old name then runs `free_chain(c)` and frees the new file's clusters. The same state follows a `free_chain` that stops on an error.

Fix: `remove` writes the deleted mark first and frees the chain after, so a cut leaves clusters marked used and no entry, which a free count at the next mount already tolerates; the current order loses no cluster on a cut but cross-links two files.

---

## F07 — issue #217
Title: fs-fat: the table size is not checked against the cluster count, so table entries are read and written outside the table
Labels: bug, part::fs
Body:
`parse` at `crates/fs/fat/src/boot.rs:207-255` accepts any non-zero `fat_sectors` (line 230) and derives `clusters` from the sector count (lines 234-241) without checking that the table holds `clusters + 2` entries. `entry_sector` at `crates/fs/fat/src/table.rs:36-40` computes `fat_start(0) + cluster / 128` with no upper bound, and `set_entry` at `crates/fs/fat/src/table.rs:88-100` writes that sector into every copy at `fat_start(copy) + within`. The specification places the entry of cluster N at `BPB_ResvdSecCnt + (N * 4) / BPB_BytsPerSec` and gives the table `BPB_FATSz32` sectors (Microsoft FAT32 File System Specification 1.03, "FAT Data Structure"); an entry beyond that is not in the table.

Trigger: a boot sector with 32 reserved sectors, 2 tables of 1 sector, 1 sector per cluster and 70000 sectors: `clusters` is 69966, above `MIN_CLUSTERS`, so `parse` succeeds. The data start is sector 34, which is cluster 2, the root directory. The entry of cluster 256 lies at sector `32 + 256 / 128 = 34`: `count_free` at `crates/fs/fat/src/table.rs:202-222` counts the root directory's bytes as table entries, and `allocate` of cluster 256 (`crates/fs/fat/src/fs.rs:207-212`, `claim` at lines 710-715) writes a table sector over the root directory and, for copy 1, over cluster 3.

Fix: `parse` refuses with `Error::Layout` a volume whose `fat_sectors * 128` is below `clusters + 2`; clamping `clusters` to what the table describes instead hides the disagreement between the two fields.

---

## F08 — issue #218
Title: fs-fat: a boot sector that claims more sectors than the device has is mounted
Labels: bug, part::fs
Body:
`parse` at `crates/fs/fat/src/boot.rs:228-241` takes `sectors` from the boot sector and `mount` at `crates/fs/fat/src/fs.rs:94-105` never compares it with `device.sectors()`. `Geometry::holds` at `crates/fs/fat/src/boot.rs:136-138` then accepts clusters whose sectors lie beyond the device. `BPB_TotSec32` is the count of all sectors in all regions of the volume (Microsoft FAT32 File System Specification 1.03, "Boot Sector and BPB").

Trigger: a partition of 70000 sectors whose boot sector claims 140000 with 32 reserved sectors, 2 tables of 1094 sectors, 1 sector per cluster: `clusters` is 137780, the data start is sector 2220, and the device holds 67780 of those clusters. The table sectors all lie on the device, so `count_free` at `crates/fs/fat/src/table.rs:202-222` succeeds and `free_clusters` counts 70000 clusters the device does not have. Once the real clusters are taken, `allocate` hands out cluster 70000: `claim` at `crates/fs/fat/src/fs.rs:710-715` writes its table entry, then the data write at `crates/fs/fat/src/fs.rs:529` or `zero_cluster` at `crates/fs/fat/src/fs.rs:718-725` reaches sector 72218 and is refused by `Partition::at` at `crates/user/servers/fs/src/partition.rs:51-53`. The cluster stays claimed with no chain naming it, and `free` is one lower than the table, once per attempt.

Fix: `mount` refuses with `Error::TooSmall(device.sectors())` a geometry whose `sectors` exceed `device.sectors()`; clamping the cluster count to the device instead lets the boot sector and the volume disagree silently.

---

## F09 — issue #219
Title: server-fs: a client that lists a root directory keeps a client table for as long as the server runs
Labels: bug, part::fs
Body:
`root_walk` at `crates/user/servers/fs/src/open.rs:194-203` makes a table for the badge and fills `roots[index]`, and no path sets it back to `None`: `is_idle` at `crates/user/servers/fs/src/open.rs:122-125` requires every root walk to be `None`, `remove` at `crates/user/servers/fs/src/open.rs:217-229` forgets a table only when it is idle, and `Close` of `ROOT` or `BOOT` is refused because `index_of` at `crates/user/servers/fs/src/open.rs:265-268` answers `None` for handles below `FIRST_FILE` (`crates/user/servers/fs/src/serve.rs:386-391`). Risk 7 at `docs/15-the-disk-on-the-machine.md:827` names clients that end without closing; this is a client that closes everything.

Trigger: 16 clients each send one `ReadDir { dir: ROOT, cursor: START }` and nothing else. The 17th client's `Open` reaches `table` at `crates/user/servers/fs/src/open.rs:153-167`, finds no free slot, and is answered `OutOfHandles`; the same for its `ReadDir` of the root. The state lasts until the server restarts.

Fix: `read_dir` drops the root walk when `next_entry` answers `None` at `crates/user/servers/fs/src/serve.rs:314-316`, `close` drops it for `ROOT` and `BOOT`, and both forget the table once it is idle; keeping the walk as it is holds the table for a listing the client never finishes, which `Close` of the root then ends.

---

## F10 — issue #220
Title: server-fs: create writes the entry before the client is known to have a free slot
Labels: bug, part::fs
Body:
`create` at `crates/user/servers/fs/src/serve.rs:214-230` calls `fs.create_dir` or `fs.create` at lines 215 and 225 and `clients.insert` at line 230 afterwards. `insert` at `crates/user/servers/fs/src/open.rs:181-187` answers `None` for a client with `MAX_OPEN` handles or a new client when `MAX_CLIENTS` tables are in use.

Trigger: a client holding 16 files sends `Create GA`; the entry is written (`crates/fs/fat/src/fs.rs:357-370`) and the reply is `OutOfHandles`. The client closes one file and sends `Create GA` again: `AlreadyExists`. The test at `crates/user/servers/fs/src/tests/serve.rs:600-632` exercises the first reply only.

Fix: `Clients` gains a `has_room(badge)` check that `create` calls before touching the volume; removing the entry after a full table instead is a second directory walk that can itself fail.

---

## F11 — issue #221
Title: server-fs: the crate's tests do not compile on their own
Labels: bug, part::fs
Body:
`crates/user/servers/fs/Cargo.toml:15-23` lists `fs-fat` without the `test-doubles` feature and no `fs-fat` dev-dependency, while `crates/user/servers/fs/src/tests/support.rs:7`, `crates/user/servers/fs/src/tests/serve.rs:11` and `crates/user/servers/fs/src/tests/volume.rs:7` import `fs_fat::doubles::RamDisk`, which `crates/fs/fat/src/lib.rs:11-12` compiles only under `test` or `test-doubles`.

`cargo test -p server-fs` fails with three `E0432: unresolved import fs_fat::doubles` errors. `cargo test -p server-fs -p fs-gpt` passes 43 tests because `crates/fs/gpt/Cargo.toml:16-18` enables the feature and resolver 3 (`Cargo.toml:5`) unifies it. The row at `docs/05-code-organization.md:193` lists `test-support` as the only dev-dependency.

Fix: add `fs-fat = { workspace = true, features = ["test-doubles"] }` under `[dev-dependencies]` as `fs-gpt` does, and name it in the docs/05 row.

---

## F12 — issue #222
Title: fs-gpt: the documentation of read claims every check of UEFI section 5.3.2, and the alternate header is not checked
Labels: bug, part::fs
Body:
The doc comment at `crates/fs/gpt/src/table.rs:24-33` states that every check UEFI 2.11 section 5.3.2 asks for is made. `read` at `crates/fs/gpt/src/table.rs:52-56` returns the primary header as soon as it parses and reads the last block only when the primary is refused. Section 5.3.2 lists, for the primary table at LBA 1, "Check the AlternateLBA to see if it is a valid GPT" (`docs/uefi/UEFI_Spec_Final_2.11.pdf`, section 5.3.2, after table 5.5).

A disk whose backup header is corrupt is read without notice, and the specification's rule that software restores an invalid backup never has an occasion in this crate.

Fix: reword the comment to name the check not made, since a reader that restores nothing has no use for the backup's validity; making the check instead costs one block read plus O(A) block reads for the backup array's checksum on every `read`.

---

## F13 — issue #223
Title: fs-fat: one entry with a foreign name byte or an invalid date makes the rest of its directory unreachable
Labels: enhancement, part::fs
Body:
`decode` at `crates/fs/fat/src/dir.rs:150-160` returns `Error::EntryName` from `Name::from_entry` (`crates/fs/fat/src/name.rs:72-82`, `allowed` at lines 120-131 accepts ASCII upper case, digits and 16 punctuation bytes) and `Error::Time` from `time::from_entry` (`crates/fs/fat/src/time.rs:51-60`). `next_entry` at `crates/fs/fat/src/fs.rs:286-287` decodes before `step`, so the cursor stays on the failing slot. The specification allows bytes above 0x7F in `DIR_Name` and `0x05` as its first byte (Microsoft FAT32 File System Specification 1.03, "FAT Directory Structure").

Trigger: a directory written by another system whose third entry has a code page byte in its name, or a `DIR_WrtDate` of 0. `find` for every name behind it answers `EntryName` or `Time`, mapped to `InvalidState` at `crates/user/servers/fs/src/error.rs:47-48`; `ReadDir` answers `InvalidState` at that cursor on every retry, and no cursor reaches the entries behind it (`crates/user/servers/fs/src/serve.rs:304-316`).

Fix: `decode` answers `Slot::Skip` for an entry whose name or date it cannot carry, and `crates/fs/fat/README.md:45-49` says so; keeping the refusal as documented leaves every entry behind the foreign one unreachable.

---

## F14 — issue #224
Title: fs-fat: the directory walk reads one sector per slot
Labels: enhancement, part::fs
Body:
`next_entry` at `crates/fs/fat/src/fs.rs:274-279` reads the sector of every slot, and `Entries` at `crates/fs/fat/src/fs.rs:45-52` keeps no sector between calls. `insert` at `crates/fs/fat/src/fs.rs:656-660` reads one sector per slot as well. A sector holds 16 slots (`crates/fs/fat/src/fs.rs:729`).

A directory of N slots costs O(N) device reads per `find`, `open`, `create`, `remove` and listing, where O(N/16) reads carry the same bytes; `open` in the server walks twice (F17). `fs-gpt` keeps the loaded block in its `Cursor` at `crates/fs/gpt/src/table.rs:96-127` for the same reason.

Fix: `Entries` carries the last sector read and its number, and `next_entry` reads again only when the slot's sector differs; `insert` does the same with a local buffer.

---

## F15 — issue #225
Title: fs-gpt: the usable range of a header is not bounded by the device
Labels: enhancement, part::fs
Body:
`Header::parse` at `crates/fs/gpt/src/header.rs:126-129` compares `first_usable` and `last_usable` only with each other, and `check_array_range` at `crates/fs/gpt/src/header.rs:188-203` bounds the array alone. UEFI 2.11 section 5.3.1 places the backup entry array after the last usable LBA and before the backup header in the last block, so `last_usable` is below `sectors - 1` on every valid table (`docs/uefi/UEFI_Spec_Final_2.11.pdf`, section 5.3.1).

A header with `last_usable` of `u64::MAX` and an entry ending at block 2^40 passes `next_entry` at `crates/fs/gpt/src/table.rs:167-173`. `Partition::new` at `crates/user/servers/fs/src/partition.rs:33-42` refuses the entry and `mount` answers `InvalidArgument` at `crates/user/servers/fs/src/volume.rs:57-58`, the refusal of a caller's argument, where a table that disagrees with the format is `InvalidState` (`crates/user/servers/fs/src/error.rs:59-77`).

Fix: `parse` refuses with `Error::Usable` a header whose `last_usable` is at or above `sectors - 1`; the check in `Partition::new` stays as the bound of the window.

---

## F16 — issue #226
Title: fs-gpt: the entry count of a header is bounded only by the device
Labels: enhancement, part::fs
Body:
`Header::parse` at `crates/fs/gpt/src/header.rs:122` takes `entry_count` unchecked, `array_len` at `crates/fs/gpt/src/header.rs:166-168` multiplies it out, and `check_array_checksum` at `crates/fs/gpt/src/table.rs:77-94` reads every block of the array. `check_array_range` at `crates/fs/gpt/src/header.rs:194` refuses only an array that reaches past the device.

A device of 2^32 sectors with a header naming 2^25 entries of 128 bytes makes `read` perform 8388608 block reads before it answers. `is_partitioned` at `crates/user/servers/fs/src/volume.rs:33-38` runs `read` at boot on every disk, so a crafted disk stalls the server's start for the time of a 4 GiB read.

Fix: `parse` refuses with `Error::ArrayRange` an `entry_count` above a constant such as 4096, a 512 KiB array at 128 bytes, which every tool in use stays far below; the format names no maximum, only the 16384-byte minimum.

---

## F17 — issue #227
Title: server-fs: open walks the directory twice for a file
Labels: enhancement, part::fs
Body:
`open` at `crates/user/servers/fs/src/serve.rs:173-176` finds the entry with `fs.find`, then at line 188 calls `fs.open`, which runs `find` again at `crates/fs/fat/src/fs.rs:322`. Both walks read one sector per slot (F14).

Every `Open` of a file costs two passes over the directory, O(2N) reads for N slots.

Fix: `fs-fat` gives `FileSystem::open_entry(&Entry) -> Result<File, Error>` that builds the `File` from an entry already found, and `open` in the server uses it; `File`'s fields are private, so the server cannot build one itself.

---

## F18 — issue #228
Title: fs-fat: Name::is_dot has no caller outside the tests
Labels: enhancement, part::fs
Body:
`Name::is_dot` at `crates/fs/fat/src/name.rs:90-94` is public and called only at `crates/fs/fat/src/tests/directory.rs:78-79`. `decode` skips dot entries by their first byte at `crates/fs/fat/src/dir.rs:146` and never asks the name.

The method is API surface with no user.

Fix: remove `is_dot` and its test, or give `decode` the call and drop the byte check at `crates/fs/fat/src/dir.rs:146`.
