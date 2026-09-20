<!-- SPDX-License-Identifier: AGPL-3.0-only -->
<!-- Copyright (C) 2026 Manuel Baesler and contributors -->

# server-fs

What the file system server decides, with no system call in it: which open
files a client holds, what one request of the file protocol does to a
FAT32 volume, and which refusal of `fs-fat` becomes which refusal of the
protocol.

A client is the badge of the capability its message came through. Badge
zero names nobody, so a request under it is refused: the open files of
every program that found the server by name would otherwise be one table
(D-185).

The process is a binary of `user-programs`, which brings the disk, the
endpoint and the clock. This crate is the part that can be tested on the
host over `fs_fat::doubles::RamDisk`.
