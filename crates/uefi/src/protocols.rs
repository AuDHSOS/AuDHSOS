// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

//! The protocols the loader opens, and the identifiers that name them.

use core::ffi::c_void;

use crate::status::Status;
use crate::tables::SystemTable;
use crate::types::{Guid, Handle};

/// `EFI_ACPI_20_TABLE_GUID`.
pub const ACPI_20_TABLE: Guid = Guid::new(
    0x8868_E871,
    0xE4F1,
    0x11D3,
    [0xBC, 0x22, 0x00, 0x80, 0xC7, 0x3C, 0x88, 0x81],
);

/// `EFI_LOADED_IMAGE_PROTOCOL_GUID`.
pub const LOADED_IMAGE_PROTOCOL: Guid = Guid::new(
    0x5B1B_31A1,
    0x9562,
    0x11D2,
    [0x8E, 0x3F, 0x00, 0xA0, 0xC9, 0x69, 0x72, 0x3B],
);

/// `EFI_SIMPLE_FILE_SYSTEM_PROTOCOL_GUID`.
pub const SIMPLE_FILE_SYSTEM_PROTOCOL: Guid = Guid::new(
    0x964E_5B22,
    0x6459,
    0x11D2,
    [0x8E, 0x39, 0x00, 0xA0, 0xC9, 0x69, 0x72, 0x3B],
);

/// `EFI_FILE_INFO_ID`.
pub const FILE_INFO: Guid = Guid::new(
    0x0957_6E92,
    0x6D3F,
    0x11D2,
    [0x8E, 0x39, 0x00, 0xA0, 0xC9, 0x69, 0x72, 0x3B],
);

/// Open mode: read.
pub const FILE_MODE_READ: u64 = 0x0000_0000_0000_0001;

/// Open mode: write.
pub const FILE_MODE_WRITE: u64 = 0x0000_0000_0000_0002;

/// Length of the fixed part of [`FileInfo`], before the name.
pub const FILE_INFO_HEADER_LEN: usize = 80;

/// Opens the volume a file system protocol serves.
pub type OpenVolume = unsafe extern "efiapi" fn(
    protocol: *mut SimpleFileSystemProtocol,
    root: *mut *mut FileProtocol,
) -> Status;

/// Opens a file relative to a directory.
pub type FileOpen = unsafe extern "efiapi" fn(
    file: *mut FileProtocol,
    new: *mut *mut FileProtocol,
    name: *const u16,
    open_mode: u64,
    attributes: u64,
) -> Status;

/// Closes a file.
pub type FileClose = unsafe extern "efiapi" fn(file: *mut FileProtocol) -> Status;

/// Reads from a file.
pub type FileRead =
    unsafe extern "efiapi" fn(file: *mut FileProtocol, size: *mut usize, buffer: *mut u8) -> Status;

/// Reads a file's metadata.
pub type FileGetInfo = unsafe extern "efiapi" fn(
    file: *mut FileProtocol,
    information: *const Guid,
    size: *mut usize,
    buffer: *mut u8,
) -> Status;

/// Writes a null-terminated UTF-16 string to the console.
pub type OutputString = unsafe extern "efiapi" fn(
    protocol: *mut SimpleTextOutputProtocol,
    string: *const u16,
) -> Status;

/// `EFI_LOADED_IMAGE_PROTOCOL`.
#[repr(C)]
pub struct LoadedImageProtocol {
    /// Version of the protocol.
    pub revision: u32,
    /// Handle of the image that loaded this one.
    pub parent_handle: Handle,
    /// The system table.
    pub system_table: *const SystemTable,
    /// Handle of the device the image was loaded from.
    pub device_handle: Handle,
    /// Path of the image on that device.
    pub file_path: *mut c_void,
    /// Reserved.
    pub reserved: *mut c_void,
    /// Size of the load options in bytes.
    pub load_options_size: u32,
    /// The load options.
    pub load_options: *mut c_void,
    /// Where the image was loaded.
    pub image_base: *mut c_void,
    /// How large the image is.
    pub image_size: u64,
    /// Memory type of the image's code.
    pub image_code_type: u32,
    /// Memory type of the image's data.
    pub image_data_type: u32,
    /// `Unload`.
    pub unload: usize,
}

/// `EFI_SIMPLE_FILE_SYSTEM_PROTOCOL`.
#[repr(C)]
pub struct SimpleFileSystemProtocol {
    /// Version of the protocol.
    pub revision: u64,
    /// `OpenVolume`.
    pub open_volume: OpenVolume,
}

/// `EFI_FILE_PROTOCOL`.
#[repr(C)]
pub struct FileProtocol {
    /// Version of the protocol.
    pub revision: u64,
    /// `Open`.
    pub open: FileOpen,
    /// `Close`.
    pub close: FileClose,
    /// `Delete`.
    pub delete: usize,
    /// `Read`.
    pub read: FileRead,
    /// `Write`.
    pub write: usize,
    /// `GetPosition`.
    pub get_position: usize,
    /// `SetPosition`.
    pub set_position: usize,
    /// `GetInfo`.
    pub get_info: FileGetInfo,
    /// `SetInfo`.
    pub set_info: usize,
    /// `Flush`.
    pub flush: usize,
    /// `OpenEx`.
    pub open_ex: usize,
    /// `ReadEx`.
    pub read_ex: usize,
    /// `WriteEx`.
    pub write_ex: usize,
    /// `FlushEx`.
    pub flush_ex: usize,
}

/// `EFI_TIME`.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
#[repr(C)]
pub struct Time {
    /// Year.
    pub year: u16,
    /// Month, counted from one.
    pub month: u8,
    /// Day, counted from one.
    pub day: u8,
    /// Hour.
    pub hour: u8,
    /// Minute.
    pub minute: u8,
    /// Second.
    pub second: u8,
    /// Zero.
    pub pad1: u8,
    /// Nanosecond.
    pub nanosecond: u32,
    /// Offset from coordinated universal time in minutes.
    pub time_zone: i16,
    /// Daylight saving flags.
    pub daylight: u8,
    /// Zero.
    pub pad2: u8,
}

/// `EFI_FILE_INFO`, without the variable-length name that follows it.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
#[repr(C)]
pub struct FileInfo {
    /// Size of the whole structure including the name.
    pub size: u64,
    /// Size of the file's content.
    pub file_size: u64,
    /// Space the file occupies on the device.
    pub physical_size: u64,
    /// When the file was created.
    pub create_time: Time,
    /// When the file was last read.
    pub last_access_time: Time,
    /// When the file was last written.
    pub modification_time: Time,
    /// The file's attributes.
    pub attribute: u64,
}

/// `EFI_SIMPLE_TEXT_OUTPUT_PROTOCOL`.
#[repr(C)]
pub struct SimpleTextOutputProtocol {
    /// `Reset`.
    pub reset: usize,
    /// `OutputString`.
    pub output_string: OutputString,
    /// `TestString`.
    pub test_string: usize,
    /// `QueryMode`.
    pub query_mode: usize,
    /// `SetMode`.
    pub set_mode: usize,
    /// `SetAttribute`.
    pub set_attribute: usize,
    /// `ClearScreen`.
    pub clear_screen: usize,
    /// `SetCursorPosition`.
    pub set_cursor_position: usize,
    /// `EnableCursor`.
    pub enable_cursor: usize,
    /// The current mode.
    pub mode: *mut c_void,
}
