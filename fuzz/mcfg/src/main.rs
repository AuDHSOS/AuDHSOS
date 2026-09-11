// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

//! The memory mapped configuration table against arbitrary bytes: no input
//! may panic, the walk over the allocations must always end, and a table
//! the parser accepts must stay inside the fixed capacity of the kernel and
//! name only windows the kernel can map.
//!
//! The bytes are tried twice: as they are, so that the checks of signature,
//! length, and checksum are exercised, and once with those three repaired,
//! so that the fuzzer reaches the allocations without having to guess a
//! checksum.

use kernel_acpi::mcfg::{MAX_ECAM_ALLOCATIONS, MCFG_HEADER_LEN, MCFG_SIGNATURE, parse};

fuzz_support::fuzz_target!(|bytes: &[u8]| {
    check(bytes);
    if let Some(repaired) = repair(bytes) {
        check(&repaired);
    }
});

/// Parses `bytes` and checks what an accepted table promises.
fn check(bytes: &[u8]) {
    let Ok(mcfg) = parse(bytes) else {
        return;
    };
    assert!(
        mcfg.count() <= MAX_ECAM_ALLOCATIONS,
        "an accepted table names more windows than the kernel holds"
    );
    for window in mcfg.allocations.iter().flatten() {
        assert!(
            window.first_bus <= window.last_bus,
            "an accepted window covers a bus range that runs backwards"
        );
        assert!(
            window.len() >= u64::from(window.buses()),
            "an accepted window covers fewer bytes than it has buses"
        );
        assert_eq!(
            window.base.as_u64() & 0xFFF,
            0,
            "an accepted window starts inside a page"
        );
    }
    if mcfg.count() > 0 {
        assert!(
            mcfg.first().is_some(),
            "a table that names a window answers with none"
        );
    }
}

/// The bytes with the signature, the length, and the checksum of a table
/// written over them, so that the fuzzer spends its time on the
/// allocations. `None` for bytes too short to hold the fixed part.
fn repair(bytes: &[u8]) -> Option<Vec<u8>> {
    if bytes.len() < MCFG_HEADER_LEN || u32::try_from(bytes.len()).is_err() {
        return None;
    }
    let mut table = bytes.to_vec();
    table.get_mut(..4)?.copy_from_slice(&MCFG_SIGNATURE);
    let length = u32::try_from(table.len()).ok()?;
    table.get_mut(4..8)?.copy_from_slice(&length.to_le_bytes());
    *table.get_mut(9)? = 0;
    let sum = table.iter().fold(0u8, |sum, byte| sum.wrapping_add(*byte));
    *table.get_mut(9)? = 0u8.wrapping_sub(sum);
    Some(table)
}
