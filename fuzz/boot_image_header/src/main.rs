// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

//! The boot image header against arbitrary bytes. The image length and the
//! memory of the machine come out of the input too, so that the
//! cross-field checks are reached with values the fuzzer chooses.


use audhsos_abi::boot_image::BootImageHeader;

fuzz_support::fuzz_target!(|bytes: &[u8]| {
    let (lengths, header) = bytes.split_at(bytes.len().min(16));
    let mut words = [0u64; 2];
    for (slot, chunk) in words.iter_mut().zip(lengths.chunks(8)) {
        let mut value = [0u8; 8];
        value[..chunk.len()].copy_from_slice(chunk);
        *slot = u64::from_le_bytes(value);
    }
    if let Ok(parsed) = BootImageHeader::parse(header, words[0], words[1]) {
        let _ = parsed.to_bytes();
    }
});
