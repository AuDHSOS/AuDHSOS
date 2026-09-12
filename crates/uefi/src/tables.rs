// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

//! The system table, the boot services table, and the runtime services
//! table.
//!
//! Invariant: each table holds every service slot in specification order,
//! so that the offset of every service is right even though the loader
//! calls only six of them.

use core::ffi::c_void;

use crate::memory_map::{AllocateType, MemoryType};
use crate::protocols::{SimpleTextOutputProtocol, Time};
use crate::status::Status;
use crate::types::{ConfigurationTable, Guid, Handle, TableHeader};

/// Signature of the system table: `IBI SYST`.
pub const SYSTEM_TABLE_SIGNATURE: u64 = 0x5453_5953_2049_4249;

/// Signature of the boot services table: `BOOTSERV`.
pub const BOOT_SERVICES_SIGNATURE: u64 = 0x5652_4553_544F_4F42;

/// Signature of the runtime services table: `RUNTSERV`. UEFI 2.11,
/// section 4.5.1, `EFI_RUNTIME_SERVICES_SIGNATURE`.
pub const RUNTIME_SERVICES_SIGNATURE: u64 = 0x5652_4553_544E_5552;

/// Allocates physically contiguous pages.
pub type AllocatePages = unsafe extern "efiapi" fn(
    allocate_type: AllocateType,
    memory_type: MemoryType,
    pages: usize,
    memory: *mut u64,
) -> Status;

/// Gives pages back.
pub type FreePages = unsafe extern "efiapi" fn(memory: u64, pages: usize) -> Status;

/// Copies the current memory map into a buffer.
pub type GetMemoryMap = unsafe extern "efiapi" fn(
    map_size: *mut usize,
    map: *mut u8,
    map_key: *mut usize,
    descriptor_size: *mut usize,
    descriptor_version: *mut u32,
) -> Status;

/// Returns the interface of a protocol a handle carries.
pub type HandleProtocol = unsafe extern "efiapi" fn(
    handle: Handle,
    protocol: *const Guid,
    interface: *mut *mut c_void,
) -> Status;

/// Ends the boot services phase.
pub type ExitBootServices = unsafe extern "efiapi" fn(image: Handle, map_key: usize) -> Status;

/// Returns the first interface of a protocol in the system.
pub type LocateProtocol = unsafe extern "efiapi" fn(
    protocol: *const Guid,
    registration: *mut c_void,
    interface: *mut *mut c_void,
) -> Status;

/// Reads the platform clock. UEFI 2.11, section 8.3.1. `capabilities` may
/// be null, which is what the loader passes: it wants the time, not the
/// resolution of the device that keeps it.
pub type GetTime = unsafe extern "efiapi" fn(time: *mut Time, capabilities: *mut c_void) -> Status;

/// The runtime services table. UEFI 2.11, section 4.5.1. Every slot the
/// loader does not call is a `usize`, so that the table keeps its size and
/// every offset stays right.
///
/// The loader calls one of the fourteen, and calls it while the boot
/// services are still up: after `ExitBootServices` a runtime service needs
/// the virtual address map the kernel never sets.
#[repr(C)]
pub struct RuntimeServices {
    /// The table header.
    pub header: TableHeader,
    /// `GetTime`.
    pub get_time: GetTime,
    /// `SetTime`.
    pub set_time: usize,
    /// `GetWakeupTime`.
    pub get_wakeup_time: usize,
    /// `SetWakeupTime`.
    pub set_wakeup_time: usize,
    /// `SetVirtualAddressMap`.
    pub set_virtual_address_map: usize,
    /// `ConvertPointer`.
    pub convert_pointer: usize,
    /// `GetVariable`.
    pub get_variable: usize,
    /// `GetNextVariableName`.
    pub get_next_variable_name: usize,
    /// `SetVariable`.
    pub set_variable: usize,
    /// `GetNextHighMonotonicCount`.
    pub get_next_high_monotonic_count: usize,
    /// `ResetSystem`.
    pub reset_system: usize,
    /// `UpdateCapsule`.
    pub update_capsule: usize,
    /// `QueryCapsuleCapabilities`.
    pub query_capsule_capabilities: usize,
    /// `QueryVariableInfo`.
    pub query_variable_info: usize,
}

/// The boot services table. Every slot the loader does not call is a
/// `usize`, so that the table keeps its size and every offset stays right.
#[repr(C)]
pub struct BootServices {
    /// The table header.
    pub header: TableHeader,
    /// `RaiseTPL`.
    pub raise_tpl: usize,
    /// `RestoreTPL`.
    pub restore_tpl: usize,
    /// `AllocatePages`.
    pub allocate_pages: AllocatePages,
    /// `FreePages`.
    pub free_pages: FreePages,
    /// `GetMemoryMap`.
    pub get_memory_map: GetMemoryMap,
    /// `AllocatePool`.
    pub allocate_pool: usize,
    /// `FreePool`.
    pub free_pool: usize,
    /// `CreateEvent`.
    pub create_event: usize,
    /// `SetTimer`.
    pub set_timer: usize,
    /// `WaitForEvent`.
    pub wait_for_event: usize,
    /// `SignalEvent`.
    pub signal_event: usize,
    /// `CloseEvent`.
    pub close_event: usize,
    /// `CheckEvent`.
    pub check_event: usize,
    /// `InstallProtocolInterface`.
    pub install_protocol_interface: usize,
    /// `ReinstallProtocolInterface`.
    pub reinstall_protocol_interface: usize,
    /// `UninstallProtocolInterface`.
    pub uninstall_protocol_interface: usize,
    /// `HandleProtocol`.
    pub handle_protocol: HandleProtocol,
    /// Reserved.
    pub reserved: usize,
    /// `RegisterProtocolNotify`.
    pub register_protocol_notify: usize,
    /// `LocateHandle`.
    pub locate_handle: usize,
    /// `LocateDevicePath`.
    pub locate_device_path: usize,
    /// `InstallConfigurationTable`.
    pub install_configuration_table: usize,
    /// `LoadImage`.
    pub load_image: usize,
    /// `StartImage`.
    pub start_image: usize,
    /// `Exit`.
    pub exit: usize,
    /// `UnloadImage`.
    pub unload_image: usize,
    /// `ExitBootServices`.
    pub exit_boot_services: ExitBootServices,
    /// `GetNextMonotonicCount`.
    pub get_next_monotonic_count: usize,
    /// `Stall`.
    pub stall: usize,
    /// `SetWatchdogTimer`.
    pub set_watchdog_timer: usize,
    /// `ConnectController`.
    pub connect_controller: usize,
    /// `DisconnectController`.
    pub disconnect_controller: usize,
    /// `OpenProtocol`.
    pub open_protocol: usize,
    /// `CloseProtocol`.
    pub close_protocol: usize,
    /// `OpenProtocolInformation`.
    pub open_protocol_information: usize,
    /// `ProtocolsPerHandle`.
    pub protocols_per_handle: usize,
    /// `LocateHandleBuffer`.
    pub locate_handle_buffer: usize,
    /// `LocateProtocol`.
    pub locate_protocol: LocateProtocol,
    /// `InstallMultipleProtocolInterfaces`.
    pub install_multiple_protocol_interfaces: usize,
    /// `UninstallMultipleProtocolInterfaces`.
    pub uninstall_multiple_protocol_interfaces: usize,
    /// `CalculateCrc32`.
    pub calculate_crc32: usize,
    /// `CopyMem`.
    pub copy_mem: usize,
    /// `SetMem`.
    pub set_mem: usize,
    /// `CreateEventEx`.
    pub create_event_ex: usize,
}

/// The system table the firmware hands to the image.
#[repr(C)]
pub struct SystemTable {
    /// The table header.
    pub header: TableHeader,
    /// Name of the firmware vendor, as a null-terminated UTF-16 string.
    pub firmware_vendor: *mut u16,
    /// Version of the firmware.
    pub firmware_revision: u32,
    /// Handle of the active console input device.
    pub console_in_handle: Handle,
    /// The console input protocol.
    pub console_in: *mut c_void,
    /// Handle of the active console output device.
    pub console_out_handle: Handle,
    /// The console output protocol.
    pub console_out: *mut SimpleTextOutputProtocol,
    /// Handle of the standard error device.
    pub standard_error_handle: Handle,
    /// The standard error protocol.
    pub standard_error: *mut SimpleTextOutputProtocol,
    /// The runtime services table.
    pub runtime_services: *mut RuntimeServices,
    /// The boot services table.
    pub boot_services: *mut BootServices,
    /// Number of entries in the configuration table.
    pub table_entries: usize,
    /// The configuration table.
    pub configuration_table: *mut ConfigurationTable,
}
