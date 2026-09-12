// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

//! The configuration space of a bus against arbitrary bytes: the walk must
//! end with functions or with an error, it must never read outside the
//! bytes it was given, and every capability list it accepts must be one
//! that ends.
//!
//! The bytes become the two hundred and fifty-six bytes of up to four
//! functions, which is what a machine answers with; everything above them
//! reads as zero, as it does on the machine this was recorded from.

use pci::address::Window;
use pci::bar::probe;
use pci::capability::walk as capabilities;
use pci::doubles::{RECORDED_LEN, RecordedConfigSpace};
use pci::enumerate::walk;
use pci::header::read;
use pci::virtio::structures;

/// How many functions the bytes are spread over.
const FUNCTIONS: usize = 4;

fuzz_support::fuzz_target!(|bytes: &[u8]| {
    let mut space = RecordedConfigSpace::blank();
    for index in 0..FUNCTIONS {
        let at = index * RECORDED_LEN;
        let mut function = [0u8; RECORDED_LEN];
        let taken = bytes.get(at..).unwrap_or(&[]);
        let len = taken.len().min(RECORDED_LEN);
        function
            .get_mut(..len)
            .unwrap_or(&mut [])
            .copy_from_slice(taken.get(..len).unwrap_or(&[]));
        let masks = [0u32; 6];
        assert!(
            space.install(u8::try_from(index).unwrap_or(0), 0, function, masks),
            "the double holds four functions"
        );
    }
    let window = Window::new(0, 0, 0).expect("one bus is a range");
    let mut addresses = Vec::new();
    let Ok(count) = walk(&space, window, |function| addresses.push(function.address)) else {
        return;
    };
    assert_eq!(count, addresses.len(), "the walk counted what it reported");
    assert!(count <= 32 * 8, "the walk left the bus it was given");
    for address in addresses {
        let Ok(Some(header)) = read(&space, address) else {
            continue;
        };
        let Ok(found) = capabilities(&space, address, &header) else {
            continue;
        };
        let _ = structures(&space, address, &found);
        let _ = probe(&mut space, address);
    }
});
