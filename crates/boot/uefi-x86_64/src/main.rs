// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

#![no_std]
#![no_main]
#![allow(unsafe_code)]
#![deny(unsafe_op_in_unsafe_fn)]
#![doc = include_str!("../README.md")]

mod bootinfo;
mod entry;
mod exit;
mod files;
mod firmware;
mod graphics;
mod loader;
mod memory;
mod paging;
mod placement;

use core::panic::PanicInfo;

use audhsos_uefi::status::Status;
use audhsos_uefi::tables::SystemTable;
use audhsos_uefi::types::Handle;

use crate::firmware::Firmware;

/// The entry point the firmware calls.
///
/// It never returns: either the kernel takes over, or the loader ends the
/// machine through the exit device. The return type is the one the
/// firmware expects.
#[unsafe(no_mangle)]
#[expect(
    clippy::not_unsafe_ptr_arg_deref,
    reason = "the firmware fixes the signature of the entry point; the pointer it passes is checked in `Firmware::new`"
)]
pub extern "efiapi" fn efi_main(image: Handle, system_table: *const SystemTable) -> Status {
    // SAFETY: the firmware calls the entry point of the image it loaded
    // with that image's handle and its own system table, which stays valid
    // until the loader leaves the boot services.
    let Ok(firmware) = (unsafe { Firmware::new(image, system_table) }) else {
        exit::die();
    };
    let failure = loader::run(&firmware);
    exit::fail_with(&firmware, format_args!("{failure}"))
}

/// A panic in the loader is a defect; the machine ends with the loader
/// failure status, because nothing is left to report through.
#[panic_handler]
fn panic(_info: &PanicInfo<'_>) -> ! {
    exit::die()
}
