// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

//! Writing the disk image the machine boots from: a GUID partition table
//! with one EFI system partition, a FAT32 file system in it, and the boot
//! image the kernel reads.

pub(crate) mod boot_image;
pub(crate) mod crc32;
pub(crate) mod disk;
pub(crate) mod fat32;
pub(crate) mod gpt;
