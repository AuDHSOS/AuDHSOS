// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

//! The boot image: the header of `audhsos-abi`, the root task as a flat
//! binary, and the archive.
//!
//! Invariant: the image this module writes is one the kernel's own parser
//! accepts, which the tests check by parsing what was written.

use audhsos_abi::boot_image::{BOOT_IMAGE_HEADER_LEN, BootImageHeader};
use audhsos_abi::layout::PAGE_SIZE;

use crate::error::Error;

/// Offset of the root task; the header occupies the first frame.
pub(crate) const ROOT_TASK_OFFSET: u64 = PAGE_SIZE;

/// The byte a placeholder root task is filled with: `hlt`.
pub(crate) const PLACEHOLDER_BYTE: u8 = 0xF4;

/// A root task that halts, for the phases in which the kernel does not
/// start one yet.
pub(crate) fn placeholder_root_task() -> Vec<u8> {
    vec![PLACEHOLDER_BYTE; usize::try_from(PAGE_SIZE).unwrap_or(4096)]
}

/// The boot image for `root_task` and `archive`, with the kernel reserve
/// the header requests (`0` for the default).
///
/// # Errors
///
/// [`Error::Usage`] for an empty root task or a reserve size that is not
/// frame-aligned.
pub(crate) fn build(
    root_task: &[u8],
    archive: &[u8],
    kernel_reserve_size: u64,
) -> Result<Vec<u8>, Error> {
    if root_task.is_empty() {
        return Err(Error::Usage("the root task is empty".to_owned()));
    }
    if !kernel_reserve_size.is_multiple_of(PAGE_SIZE) {
        return Err(Error::Usage(format!(
            "the kernel reserve of {kernel_reserve_size} bytes is not frame-aligned"
        )));
    }
    let root_task_len = u64::try_from(root_task.len()).unwrap_or(u64::MAX);
    let archive_offset = ROOT_TASK_OFFSET
        .saturating_add(root_task_len)
        .next_multiple_of(PAGE_SIZE);
    let archive_len = u64::try_from(archive.len()).unwrap_or(u64::MAX);
    let header = BootImageHeader {
        root_task_offset: ROOT_TASK_OFFSET,
        root_task_len,
        archive_offset,
        archive_len,
        kernel_reserve_size,
    };
    let total = usize::try_from(archive_offset.saturating_add(archive_len)).unwrap_or(usize::MAX);
    let mut image = vec![0u8; total];
    copy(&mut image, 0, &header.to_bytes());
    copy(
        &mut image,
        usize::try_from(ROOT_TASK_OFFSET).unwrap_or(0),
        root_task,
    );
    copy(
        &mut image,
        usize::try_from(archive_offset).unwrap_or(0),
        archive,
    );
    debug_assert_eq!(BOOT_IMAGE_HEADER_LEN, 64);
    Ok(image)
}

fn copy(image: &mut [u8], offset: usize, bytes: &[u8]) {
    if let Some(slot) = offset
        .checked_add(bytes.len())
        .and_then(|end| image.get_mut(offset..end))
    {
        slot.copy_from_slice(bytes);
    }
}
