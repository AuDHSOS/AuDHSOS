// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

//! Concurrent counters stop touching the shared page only when it is unmapped.
#![no_std]
#![no_main]
#![allow(unsafe_code)]

use audhsos_abi as _;
use core::sync::atomic::{AtomicU64, Ordering};
use user_rt as _;
use user_sys_x86_64 as sys;

sys::entry!(main);

fn main(ipc_buffer: u64) -> ! {
    // SAFETY: this thread owns its IPC buffer before entering the loop.
    let buffer = unsafe { sys::buffer(ipc_buffer) };
    let address = buffer.reader().word(0).unwrap_or(0);
    let index = buffer.reader().word(1).unwrap_or(0);
    let address = usize::try_from(address.saturating_add(index.saturating_mul(8))).unwrap_or(0);
    // SAFETY: the test maps an aligned atomic counter for this worker.
    let counter = unsafe { &*core::ptr::without_provenance::<AtomicU64>(address) };
    loop {
        counter.fetch_add(1, Ordering::Relaxed);
    }
}

#[panic_handler]
const fn panic(_info: &core::panic::PanicInfo<'_>) -> ! {
    loop {}
}
