// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

//! The boot information page against arbitrary bytes: no input may panic,
//! and every view the parser hands back must survive being read out.


use audhsos_abi::boot_info::BootInfoView;

fuzz_support::fuzz_target!(|bytes: &[u8]| {
    if let Ok(view) = BootInfoView::parse(bytes) {
        let _ = view.header();
        let _ = view.phys_window_base();
        let _ = view.acpi_rsdp();
        let _ = view.framebuffer();
        for region in view.regions() {
            let _ = region.end();
        }
    }
});
