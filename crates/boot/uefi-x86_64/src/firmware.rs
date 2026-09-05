// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

//! The boot services, one method per service, one `unsafe` block per call.
//!
//! Invariant: every method here is the only place that calls its service,
//! and every value that leaves a method has been checked, so that the rest
//! of the loader works with typed values instead of firmware pointers.

use core::ffi::c_void;

use audhsos_uefi::memory_map::{AllocateType, MemoryType};
use audhsos_uefi::protocols::SimpleTextOutputProtocol;
use audhsos_uefi::status::Status;
use audhsos_uefi::tables::{
    BOOT_SERVICES_SIGNATURE, BootServices, SYSTEM_TABLE_SIGNATURE, SystemTable,
};
use audhsos_uefi::types::{Guid, Handle};
use audhsos_uefi::utf16;
use kernel_types::{PhysAddr, PhysFrame, PhysFrameRange};

/// Number of code units a diagnostic may occupy, terminator included.
const MESSAGE_UNITS: usize = 256;

/// What `GetMemoryMap` reported about the map it wrote.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct MemoryMapInfo {
    /// Number of bytes the map occupies.
    pub(crate) size: usize,
    /// The key `ExitBootServices` expects.
    pub(crate) key: usize,
    /// The stride between two descriptors.
    pub(crate) descriptor_size: usize,
}

/// The firmware the loader was started by.
#[derive(Clone, Copy)]
pub(crate) struct Firmware<'a> {
    image: Handle,
    table: &'a SystemTable,
    boot: &'a BootServices,
    console: Option<&'a SimpleTextOutputProtocol>,
}

impl<'a> Firmware<'a> {
    /// The firmware behind the system table the entry point was handed.
    ///
    /// # Errors
    ///
    /// [`Status::INVALID_PARAMETER`] if a pointer is null or a table
    /// carries the wrong signature.
    ///
    /// # Safety
    ///
    /// `table` must be the system table the firmware passed to the image,
    /// and it must stay valid for `'a`, which ends with the loader.
    pub(crate) unsafe fn new(image: Handle, table: *const SystemTable) -> Result<Self, Status> {
        if table.is_null() {
            return Err(Status::INVALID_PARAMETER);
        }
        // SAFETY: the caller promises that the pointer is the system table
        // the firmware passed and that it stays valid for the whole run.
        let table = unsafe { &*table };
        if table.header.signature != SYSTEM_TABLE_SIGNATURE || table.boot_services.is_null() {
            return Err(Status::INVALID_PARAMETER);
        }
        // SAFETY: the system table is valid and its boot services pointer
        // is not null, so the firmware owns a boot services table there
        // for as long as the system table lives.
        let boot = unsafe { &*table.boot_services };
        if boot.header.signature != BOOT_SERVICES_SIGNATURE {
            return Err(Status::INVALID_PARAMETER);
        }
        let console = if table.console_out.is_null() {
            None
        } else {
            // SAFETY: the system table is valid and its console pointer is
            // not null, so the firmware owns the protocol there for as
            // long as the system table lives.
            Some(unsafe { &*table.console_out })
        };
        Ok(Firmware {
            image,
            table,
            boot,
            console,
        })
    }

    /// `count` physically contiguous frames of loader data.
    ///
    /// # Errors
    ///
    /// [`Status::INVALID_PARAMETER`] for a count of zero or an address the
    /// firmware reports that is not a frame start; the status of the
    /// service otherwise.
    pub(crate) fn allocate_pages(&self, count: u64) -> Result<PhysFrameRange, Status> {
        if count == 0 {
            return Err(Status::INVALID_PARAMETER);
        }
        let pages = usize::try_from(count).map_err(|_| Status::OUT_OF_RESOURCES)?;
        let mut memory = 0u64;
        // SAFETY: the boot services table is the one the firmware handed
        // over, the memory pointer names a local variable that outlives
        // the call, and boot services are still active.
        let status = unsafe {
            (self.boot.allocate_pages)(
                AllocateType::AnyPages,
                MemoryType::LoaderData,
                pages,
                &raw mut memory,
            )
        };
        status.ok(())?;
        let start = PhysAddr::new(memory).map_err(|_| Status::INVALID_PARAMETER)?;
        let frame = PhysFrame::from_start(start).map_err(|_| Status::INVALID_PARAMETER)?;
        PhysFrameRange::new(frame, count).map_err(|_| Status::INVALID_PARAMETER)
    }

    /// Gives frames obtained from [`Firmware::allocate_pages`] back.
    pub(crate) fn free_pages(&self, range: PhysFrameRange) -> Status {
        let Ok(pages) = usize::try_from(range.count()) else {
            return Status::INVALID_PARAMETER;
        };
        // SAFETY: the range came from `allocate_pages`, which hands out
        // exactly what the firmware allocated, and boot services are still
        // active.
        unsafe { (self.boot.free_pages)(range.start().start().as_u64(), pages) }
    }

    /// Writes the current memory map into `buffer`.
    ///
    /// # Errors
    ///
    /// [`Status::BUFFER_TOO_SMALL`] if the buffer is shorter than the map;
    /// the status of the service otherwise.
    pub(crate) fn memory_map(&self, buffer: &mut [u8]) -> Result<MemoryMapInfo, Status> {
        let mut size = buffer.len();
        let mut key = 0usize;
        let mut descriptor_size = 0usize;
        let mut version = 0u32;
        // SAFETY: the four out pointers name local variables that outlive
        // the call, the buffer pointer and its length come from one slice,
        // and boot services are still active.
        let status = unsafe {
            (self.boot.get_memory_map)(
                &raw mut size,
                buffer.as_mut_ptr(),
                &raw mut key,
                &raw mut descriptor_size,
                &raw mut version,
            )
        };
        status.ok(MemoryMapInfo {
            size,
            key,
            descriptor_size,
        })
    }

    /// The interface of `guid` that `handle` carries.
    ///
    /// # Errors
    ///
    /// [`Status::NOT_FOUND`] if the firmware reports success but no
    /// interface; the status of the service otherwise.
    pub(crate) fn handle_protocol<T>(&self, handle: Handle, guid: Guid) -> Result<&'a T, Status> {
        let mut interface: *mut c_void = core::ptr::null_mut();
        // SAFETY: the guid and the interface pointer name local variables
        // that outlive the call, and boot services are still active.
        let status =
            unsafe { (self.boot.handle_protocol)(handle, &raw const guid, &raw mut interface) };
        status.ok(())?;
        Self::interface(interface)
    }

    /// The first interface of `guid` in the system.
    ///
    /// # Errors
    ///
    /// [`Status::NOT_FOUND`] if the firmware reports success but no
    /// interface; the status of the service otherwise.
    pub(crate) fn locate_protocol<T>(&self, guid: Guid) -> Result<&'a T, Status> {
        let mut interface: *mut c_void = core::ptr::null_mut();
        // SAFETY: the guid and the interface pointer name local variables
        // that outlive the call, and boot services are still active.
        let status = unsafe {
            (self.boot.locate_protocol)(&raw const guid, core::ptr::null_mut(), &raw mut interface)
        };
        status.ok(())?;
        Self::interface(interface)
    }

    /// The protocol structure the firmware reported, as a reference.
    fn interface<T>(interface: *mut c_void) -> Result<&'a T, Status> {
        if interface.is_null() {
            return Err(Status::NOT_FOUND);
        }
        // SAFETY: the firmware reported success, so the pointer names a
        // protocol structure of the requested type that it owns for as
        // long as boot services run, and the loader never writes to it.
        Ok(unsafe { &*interface.cast::<T>() })
    }

    /// Ends the boot services phase. Nothing but the exit device and the
    /// memory the loader already owns is reachable afterwards.
    pub(crate) fn exit_boot_services(&self, key: usize) -> Status {
        // SAFETY: the image handle is the one the firmware passed and the
        // key comes from the memory map the caller read last.
        unsafe { (self.boot.exit_boot_services)(self.image, key) }
    }

    /// Writes a diagnostic on the console, if there is one and the text is
    /// short enough ASCII. A diagnostic that cannot be written is dropped:
    /// the loader is already reporting a failure.
    pub(crate) fn output_string(&self, text: &str) {
        let Some(console) = self.console else {
            return;
        };
        let mut buffer = [0u16; MESSAGE_UNITS];
        let Ok(encoded) = utf16::encode(text, &mut buffer) else {
            return;
        };
        let protocol = core::ptr::from_ref(console).cast_mut();
        // SAFETY: the protocol pointer names the structure the firmware
        // reported, the string is null-terminated by the encoder, and the
        // firmware reads both without writing to the protocol.
        let _ = unsafe { (console.output_string)(protocol, encoded.as_ptr()) };
    }

    /// The address of the configuration table `guid` names.
    pub(crate) fn configuration_table(&self, guid: Guid) -> Option<u64> {
        if self.table.configuration_table.is_null() || self.table.table_entries == 0 {
            return None;
        }
        // SAFETY: the system table reports how many entries the array
        // holds, and the firmware owns it for as long as the system table
        // lives.
        let entries = unsafe {
            core::slice::from_raw_parts(self.table.configuration_table, self.table.table_entries)
        };
        entries
            .iter()
            .find(|entry| entry.vendor_guid == guid)
            .and_then(|entry| u64::try_from(entry.vendor_table.addr()).ok())
            .filter(|address| *address != 0)
    }

    /// The handle of the loader image itself.
    pub(crate) const fn image(&self) -> Handle {
        self.image
    }
}
