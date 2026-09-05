// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

//! Checking the constants the linker scripts repeat against the ones the
//! ABI defines. A script cannot include a Rust constant, so the value is
//! written twice and compared here.

use std::path::Path;

use crate::error::Error;
use crate::fs;

/// The kernel's linker script.
pub(crate) const KERNEL_SCRIPT: &str = "crates/kernel/bin/kernel.ld";

/// The value of `name = <number>;` in a linker script.
pub(crate) fn constant(script: &str, name: &str) -> Option<u64> {
    for line in script.lines() {
        let line = line.trim();
        let Some(rest) = line.strip_prefix(name) else {
            continue;
        };
        let Some(rest) = rest.trim_start().strip_prefix('=') else {
            continue;
        };
        let value = rest.trim().trim_end_matches(';').trim();
        let parsed = match value
            .strip_prefix("0x")
            .or_else(|| value.strip_prefix("0X"))
        {
            Some(hex) => u64::from_str_radix(hex, 16).ok(),
            None => value.parse().ok(),
        };
        if parsed.is_some() {
            return parsed;
        }
    }
    None
}

/// Reports every constant a linker script and the ABI disagree on.
pub(crate) fn check(root: &Path) -> Result<Vec<String>, Error> {
    let script = fs::read(&root.join(KERNEL_SCRIPT))?;
    let mut violations = Vec::new();
    let expected = audhsos_abi::layout::KERNEL_BASE;
    match constant(&script, "KERNEL_BASE") {
        Some(value) if value == expected => {}
        Some(value) => violations.push(format!(
            "{KERNEL_SCRIPT}: KERNEL_BASE is {value:#x}, the ABI says {expected:#x}"
        )),
        None => violations.push(format!("{KERNEL_SCRIPT}: KERNEL_BASE is missing")),
    }
    Ok(violations)
}
