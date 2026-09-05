// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

//! The ELF parser against arbitrary bytes: no input may panic, and every
//! image the parser accepts must describe segments inside the file.


use audhsos_elf::image::{Constraints, parse};

/// The whole address space, so that the parser rejects an image for what it
/// is and not for where the caller wanted it.
const ANYWHERE: Constraints = Constraints {
    lowest_vaddr: 0,
    highest_vaddr: u64::MAX,
};

fuzz_support::fuzz_target!(|bytes: &[u8]| {
    if let Ok(image) = parse(bytes, ANYWHERE) {
        for segment in image.segments() {
            assert!(
                ANYWHERE.allows(segment.vaddr, segment.mem_size),
                "an accepted segment leaves the bounds it was parsed with"
            );
        }
    }
});
