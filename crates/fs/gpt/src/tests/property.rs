// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

//! What holds for every table, not only for the ones written by hand.

#![allow(
    clippy::arithmetic_side_effects,
    clippy::as_conversions,
    clippy::cast_possible_truncation,
    clippy::indexing_slicing
)]

use fs_fat::doubles::RamDisk;
use test_support::generators::{pair, range};
use test_support::property::check;

use crate::entry::{ESP_TYPE_GUID, Entry};
use crate::header::{FIRST_USABLE, MIN_SECTORS, last_usable};
use crate::table::{find, read, write};

use super::support::{DISK_GUID, PARTITION_GUID};

#[test]
fn a_partition_written_onto_any_device_is_the_partition_read_back() {
    let cases = pair(range(MIN_SECTORS..=8192u64), range(0..=4095u64));
    check(
        "gpt round trip",
        &cases,
        |&(sectors, offset)| -> Result<(), String> {
            let last = last_usable(sectors);
            let first = FIRST_USABLE.saturating_add(offset).min(last);
            let entry = Entry::new(ESP_TYPE_GUID, PARTITION_GUID, first, last, "EFI")
                .map_err(|error| format!("{error}"))?;
            let blocks = u32::try_from(sectors).map_err(|_| "too many blocks".to_owned())?;
            let mut device = RamDisk::new(blocks);
            write(&mut device, &DISK_GUID, &[entry]).map_err(|error| format!("{error}"))?;
            let header = read(&device).map_err(|error| format!("{error}"))?;
            if header.my_lba != 1 {
                return Err(format!("the primary was not read: {header:?}"));
            }
            match find(&device, &header, &ESP_TYPE_GUID).map_err(|error| format!("{error}"))? {
                Some(found) if found == entry => Ok(()),
                other => Err(format!("{other:?} is not {entry:?}")),
            }
        },
    );
}

#[test]
fn a_block_of_any_table_that_is_torn_is_refused_or_recovered_and_never_read_wrong() {
    let cases = pair(range(0..=2047usize * crate::SECTOR), range(0..=7u32));
    check("gpt tear", &cases, |&(offset, bit)| -> Result<(), String> {
        let sectors = 2048u32;
        let entry = Entry::new(
            ESP_TYPE_GUID,
            PARTITION_GUID,
            1024,
            last_usable(u64::from(sectors)),
            "EFI",
        )
        .map_err(|error| format!("{error}"))?;
        let mut device = RamDisk::new(sectors);
        write(&mut device, &DISK_GUID, &[entry]).map_err(|error| format!("{error}"))?;
        let Some(byte) = device.bytes_mut().get_mut(offset) else {
            return Err(format!("byte {offset} is not on the device"));
        };
        *byte ^= 1u8 << bit;
        match read(&device) {
            Err(_) => Ok(()),
            Ok(header) => match find(&device, &header, &ESP_TYPE_GUID) {
                Err(_) | Ok(None) => Ok(()),
                Ok(Some(found)) if found == entry => Ok(()),
                Ok(Some(found)) => Err(format!("{found:?} is not {entry:?}")),
            },
        }
    });
}
