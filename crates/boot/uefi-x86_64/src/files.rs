// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

//! Reading the two files the loader needs from the boot volume.
//!
//! Invariant: a file that comes back has been read completely into frames
//! the loader owns, and the length reported is the length the file system
//! reported, so that the parsers never see a partially filled buffer.

use core::fmt;

use audhsos_abi::layout::PAGE_SIZE;
use audhsos_uefi::protocols::{
    FILE_INFO, FILE_MODE_READ, FileProtocol, LOADED_IMAGE_PROTOCOL, LoadedImageProtocol,
    SIMPLE_FILE_SYSTEM_PROTOCOL, SimpleFileSystemProtocol,
};
use audhsos_uefi::status::Status;
use audhsos_uefi::utf16;
use kernel_types::PhysFrameRange;

use crate::firmware::Firmware;

/// Offset of the file size in `EFI_FILE_INFO`.
const FILE_SIZE_OFFSET: usize = 8;

/// Number of bytes the loader reserves for one file information
/// structure, name included.
const INFO_BYTES: usize = 512;

/// Number of code units a file name may occupy, terminator included.
const NAME_UNITS: usize = 64;

/// Why a file could not be read.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum LoadError {
    /// A firmware service reported a failure.
    Protocol(Status),
    /// The name is not one the firmware can be given.
    Name,
    /// The file holds no bytes.
    Empty,
    /// The file is longer than the loader can hold.
    TooLarge,
    /// The file system stopped delivering bytes before the end.
    Truncated,
}

impl fmt::Display for LoadError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            LoadError::Protocol(status) => write!(f, "{status}"),
            LoadError::Name => f.write_str("the file name cannot be encoded"),
            LoadError::Empty => f.write_str("the file is empty"),
            LoadError::TooLarge => f.write_str("the file is too large"),
            LoadError::Truncated => f.write_str("the file ended early"),
        }
    }
}

impl From<Status> for LoadError {
    fn from(status: Status) -> Self {
        LoadError::Protocol(status)
    }
}

/// A file the loader read into frames of its own.
#[derive(Clone, Copy, Debug)]
pub(crate) struct LoadedFile {
    /// The frames holding the file.
    pub(crate) frames: PhysFrameRange,
    /// The length of the file in bytes.
    pub(crate) len: u64,
}

/// The root directory of the volume the loader was started from.
#[derive(Clone, Copy)]
pub(crate) struct Volume<'a> {
    root: &'a FileProtocol,
}

/// The volume the loader image itself was loaded from.
///
/// # Errors
///
/// [`LoadError::Protocol`] if a protocol is missing or the volume cannot
/// be opened.
pub(crate) fn open_volume<'a>(firmware: &Firmware<'a>) -> Result<Volume<'a>, LoadError> {
    let image: &LoadedImageProtocol =
        firmware.handle_protocol(firmware.image(), LOADED_IMAGE_PROTOCOL)?;
    let system: &SimpleFileSystemProtocol =
        firmware.handle_protocol(image.device_handle, SIMPLE_FILE_SYSTEM_PROTOCOL)?;
    let mut root: *mut FileProtocol = core::ptr::null_mut();
    let protocol = core::ptr::from_ref(system).cast_mut();
    // SAFETY: the protocol pointer names the structure the firmware
    // reported, the out pointer names a local variable that outlives the
    // call, and boot services are still active.
    let status = unsafe { (system.open_volume)(protocol, &raw mut root) };
    status.ok(())?;
    if root.is_null() {
        return Err(LoadError::Protocol(Status::NOT_FOUND));
    }
    // SAFETY: the firmware reported success and a non-null pointer, so it
    // owns a file protocol there until the loader closes it, which it does
    // not do for the root directory.
    let root = unsafe { &*root };
    Ok(Volume { root })
}

impl Volume<'_> {
    /// Reads `name` from the volume into frames the loader owns. The name
    /// is a `\`-separated path of 8.3 names.
    ///
    /// # Errors
    ///
    /// [`LoadError`] for a name the firmware cannot be given, a missing or
    /// empty file, a file that does not fit, and every firmware failure.
    pub(crate) fn read_file(
        &self,
        firmware: &Firmware<'_>,
        name: &str,
    ) -> Result<LoadedFile, LoadError> {
        let mut units = [0u16; NAME_UNITS];
        let encoded = utf16::encode(name, &mut units).map_err(|_| LoadError::Name)?;
        let file = self.open(encoded)?;
        let outcome = read_open_file(firmware, file);
        let protocol = core::ptr::from_ref(file).cast_mut();
        // SAFETY: the protocol pointer names the file the firmware opened,
        // nothing reads it afterwards, and boot services are still active.
        let _ = unsafe { (file.close)(protocol) };
        outcome
    }

    /// Opens `name` for reading.
    fn open(&self, name: &[u16]) -> Result<&FileProtocol, LoadError> {
        let mut handle: *mut FileProtocol = core::ptr::null_mut();
        let protocol = core::ptr::from_ref(self.root).cast_mut();
        // SAFETY: the protocol pointer names the root directory the
        // firmware opened, the name is null-terminated by the encoder, the
        // out pointer names a local variable that outlives the call, and
        // boot services are still active.
        let status = unsafe {
            (self.root.open)(protocol, &raw mut handle, name.as_ptr(), FILE_MODE_READ, 0)
        };
        status.ok(())?;
        if handle.is_null() {
            return Err(LoadError::Protocol(Status::NOT_FOUND));
        }
        // SAFETY: the firmware reported success and a non-null pointer, so
        // it owns a file protocol there until the caller closes it.
        Ok(unsafe { &*handle })
    }
}

/// Reads the whole of an open file into fresh frames.
fn read_open_file(firmware: &Firmware<'_>, file: &FileProtocol) -> Result<LoadedFile, LoadError> {
    let len = file_size(file)?;
    if len == 0 {
        return Err(LoadError::Empty);
    }
    let count = len.div_ceil(PAGE_SIZE);
    let frames = firmware.allocate_pages(count)?;
    // SAFETY: the frames were just allocated for this file, nothing else
    // holds a reference to them, and the identity mapping is active.
    let buffer =
        unsafe { crate::memory::bytes_mut(frames, frames.bytes()) }.ok_or(LoadError::TooLarge)?;
    let wanted = usize::try_from(len).map_err(|_| LoadError::TooLarge)?;
    let mut done = 0usize;
    while done < wanted {
        let rest = buffer.get_mut(done..wanted).ok_or(LoadError::TooLarge)?;
        let mut chunk = rest.len();
        let protocol = core::ptr::from_ref(file).cast_mut();
        // SAFETY: the protocol pointer names the open file, the size
        // pointer names a local variable that outlives the call, the
        // buffer pointer and its length come from one slice, and boot
        // services are still active.
        let status = unsafe { (file.read)(protocol, &raw mut chunk, rest.as_mut_ptr()) };
        status.ok(())?;
        if chunk == 0 {
            return Err(LoadError::Truncated);
        }
        done = done.saturating_add(chunk.min(rest.len()));
    }
    Ok(LoadedFile { frames, len })
}

/// The length of an open file, read from its `EFI_FILE_INFO`.
fn file_size(file: &FileProtocol) -> Result<u64, LoadError> {
    let mut info = [0u8; INFO_BYTES];
    let mut size = info.len();
    let guid = FILE_INFO;
    let protocol = core::ptr::from_ref(file).cast_mut();
    // SAFETY: the protocol pointer names the open file, the guid and the
    // size pointer name local variables that outlive the call, the buffer
    // pointer and its length come from one array, and boot services are
    // still active.
    let status =
        unsafe { (file.get_info)(protocol, &raw const guid, &raw mut size, info.as_mut_ptr()) };
    status.ok(())?;
    let field = info
        .get(FILE_SIZE_OFFSET..FILE_SIZE_OFFSET.saturating_add(8))
        .and_then(|bytes| <[u8; 8]>::try_from(bytes).ok())
        .ok_or(LoadError::Truncated)?;
    Ok(u64::from_le_bytes(field))
}
