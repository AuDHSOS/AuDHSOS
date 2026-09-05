// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

//! The disk image: a GUID partition table with one EFI system partition
//! and a FAT32 file system in it.
//!
//! Invariant: the image is a whole number of sectors and grows only in
//! whole mebibytes, so that two runs with the same files produce the same
//! bytes.

use crate::error::Error;
use crate::image::{fat32, gpt};

/// The size the image has unless the files need more.
pub(crate) const DEFAULT_SIZE: u64 = 64 * 1024 * 1024;

/// The step the image grows in when the files need more room.
pub(crate) const GROWTH_STEP: u64 = 1024 * 1024;

/// Path of the loader on the boot volume.
pub(crate) const LOADER_PATH: &str = "EFI/BOOT/BOOTX64.EFI";

/// Path of the kernel on the boot volume.
pub(crate) const KERNEL_PATH: &str = "AUDHSOS/KERNEL.ELF";

/// Path of the boot image on the boot volume.
pub(crate) const BOOT_IMAGE_PATH: &str = "AUDHSOS/BOOT.IMG";

/// The disk image holding `files`, whose paths are `/`-separated 8.3
/// names.
///
/// # Errors
///
/// [`Error::Usage`] if the files do not fit into the largest image this
/// writer builds, or if a name is not an 8.3 name.
pub(crate) fn build(files: &[(&str, Vec<u8>)]) -> Result<Vec<u8>, Error> {
    let content: u64 = files
        .iter()
        .map(|(_, bytes)| u64::try_from(bytes.len()).unwrap_or(u64::MAX))
        .fold(0u64, |total, len| {
            total.saturating_add(len.next_multiple_of(GROWTH_STEP))
        });
    let mut size = DEFAULT_SIZE.max(content.saturating_add(DEFAULT_SIZE.wrapping_div(2)));
    size = size.next_multiple_of(GROWTH_STEP);
    loop {
        match try_build(files, size) {
            Ok(image) => return Ok(image),
            Err(error) if size >= DEFAULT_SIZE.saturating_mul(64) => return Err(error),
            Err(_) => size = size.saturating_add(GROWTH_STEP),
        }
    }
}

/// The disk image of exactly `size` bytes.
///
/// # Errors
///
/// The errors of the partition table and the file system writers.
pub(crate) fn try_build(files: &[(&str, Vec<u8>)], size: u64) -> Result<Vec<u8>, Error> {
    let sectors = gpt::sectors_of(size);
    let mut image = vec![0u8; usize::try_from(size).unwrap_or(usize::MAX)];
    let (first, last) = gpt::write(&mut image, sectors)?;
    let start = gpt::sector_offset(first);
    let end = gpt::sector_offset(last.saturating_add(1));
    let partition = image
        .get_mut(start..end)
        .ok_or_else(|| Error::Usage("the partition does not fit into the image".to_owned()))?;
    fat32::write(partition, files)?;
    Ok(image)
}

/// The partition of an image this module wrote. Only the tests read the
/// image back; the product only writes it.
#[cfg(test)]
pub(crate) fn partition(image: &[u8]) -> Option<&[u8]> {
    let sectors = u64::try_from(image.len() / gpt::SECTOR).unwrap_or(0);
    let (first, last) = gpt::partition_range(sectors);
    let start = gpt::sector_offset(first);
    let end = gpt::sector_offset(last.saturating_add(1));
    image.get(start..end)
}
