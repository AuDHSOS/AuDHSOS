// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

//! Tests of `crate::boot_image`, covering the catalog items 6.6.10 for the
//! boot image header.

#![allow(clippy::arithmetic_side_effects)]

use crate::boot_image::{
    BOOT_IMAGE_HEADER_LEN, BOOT_IMAGE_MAGIC, BOOT_IMAGE_VERSION, BootImageError, BootImageHeader,
};
use crate::layout::PAGE_SIZE;
use crate::strategies::{any_boot_image_bytes, any_boot_image_header, image_len_for};
use test_support::property::check;

/// Length of the image every test parses against.
const IMAGE_LEN: u64 = 16 * PAGE_SIZE;

/// Usable memory every test parses against.
const RAM: u64 = 64 * 1024 * 1024;

/// A header written field by field, so that tests can produce values the
/// serializer never writes.
#[derive(Clone, Copy, Debug)]
struct Raw {
    magic: [u8; 8],
    version: u32,
    header_len: u32,
    root_task_offset: u64,
    root_task_len: u64,
    archive_offset: u64,
    archive_len: u64,
    kernel_reserve_size: u64,
    flags: u64,
}

impl Default for Raw {
    fn default() -> Self {
        Raw {
            magic: BOOT_IMAGE_MAGIC,
            version: BOOT_IMAGE_VERSION,
            header_len: 64,
            root_task_offset: PAGE_SIZE,
            root_task_len: 2 * PAGE_SIZE,
            archive_offset: 4 * PAGE_SIZE,
            archive_len: PAGE_SIZE,
            kernel_reserve_size: 0,
            flags: 0,
        }
    }
}

impl Raw {
    fn bytes(self) -> [u8; BOOT_IMAGE_HEADER_LEN] {
        let mut bytes = [0u8; BOOT_IMAGE_HEADER_LEN];
        bytes[..8].copy_from_slice(&self.magic);
        bytes[8..12].copy_from_slice(&self.version.to_le_bytes());
        bytes[12..16].copy_from_slice(&self.header_len.to_le_bytes());
        bytes[16..24].copy_from_slice(&self.root_task_offset.to_le_bytes());
        bytes[24..32].copy_from_slice(&self.root_task_len.to_le_bytes());
        bytes[32..40].copy_from_slice(&self.archive_offset.to_le_bytes());
        bytes[40..48].copy_from_slice(&self.archive_len.to_le_bytes());
        bytes[48..56].copy_from_slice(&self.kernel_reserve_size.to_le_bytes());
        bytes[56..64].copy_from_slice(&self.flags.to_le_bytes());
        bytes
    }

    fn parse(self) -> Result<BootImageHeader, BootImageError> {
        BootImageHeader::parse(&self.bytes(), IMAGE_LEN, RAM)
    }

    fn parse_in(self, image_len: u64) -> Result<BootImageHeader, BootImageError> {
        BootImageHeader::parse(&self.bytes(), image_len, RAM)
    }
}

#[test]
fn default_header_parses_and_reports_its_fields() {
    let header = Raw::default().parse().unwrap();
    assert_eq!(header.root_task_offset, PAGE_SIZE);
    assert_eq!(header.root_task_len, 2 * PAGE_SIZE);
    assert_eq!(header.archive_offset, 4 * PAGE_SIZE);
    assert_eq!(header.archive_len, PAGE_SIZE);
    assert_eq!(header.kernel_reserve_size, 0);
}

#[test]
fn fewer_bytes_than_the_header_are_too_short() {
    let bytes = Raw::default().bytes();
    assert_eq!(
        BootImageHeader::parse(&bytes[..63], IMAGE_LEN, RAM),
        Err(BootImageError::TooShort)
    );
    assert_eq!(
        BootImageHeader::parse(&[], IMAGE_LEN, RAM),
        Err(BootImageError::TooShort)
    );
    assert!(BootImageHeader::parse(&bytes, IMAGE_LEN, RAM).is_ok());
}

#[test]
fn wrong_magic_is_rejected() {
    for magic in [*b"AUDHSOS1", [0; 8], *b"audhsos\0"] {
        let raw = Raw {
            magic,
            ..Raw::default()
        };
        assert_eq!(raw.parse(), Err(BootImageError::BadMagic));
    }
}

#[test]
fn wrong_version_is_rejected() {
    for version in [0, 2, u32::MAX] {
        let raw = Raw {
            version,
            ..Raw::default()
        };
        assert_eq!(
            raw.parse(),
            Err(BootImageError::UnsupportedVersion(version))
        );
    }
}

#[test]
fn header_length_below_the_minimum_or_beyond_the_image_is_rejected() {
    for header_len in [0, 63] {
        let raw = Raw {
            header_len,
            ..Raw::default()
        };
        assert_eq!(raw.parse(), Err(BootImageError::HeaderLength(header_len)));
    }
    let raw = Raw {
        header_len: 64,
        ..Raw::default()
    };
    assert_eq!(
        raw.parse_in(63),
        Err(BootImageError::HeaderLength(64)),
        "a header longer than the image is rejected"
    );
    assert!(raw.parse().is_ok());
}

#[test]
fn root_task_offset_unaligned_inside_the_header_or_beyond_the_image_is_rejected() {
    let unaligned = Raw {
        root_task_offset: PAGE_SIZE + 1,
        ..Raw::default()
    };
    assert_eq!(unaligned.parse(), Err(BootImageError::RootTaskOffset));
    let inside_header = Raw {
        root_task_offset: 0,
        ..Raw::default()
    };
    assert_eq!(inside_header.parse(), Err(BootImageError::RootTaskOffset));
    let beyond = Raw {
        root_task_offset: IMAGE_LEN,
        root_task_len: PAGE_SIZE,
        archive_len: 0,
        ..Raw::default()
    };
    assert_eq!(beyond.parse(), Err(BootImageError::RootTaskOffset));
}

#[test]
fn root_task_length_zero_overflowing_or_beyond_the_image_is_rejected() {
    let zero = Raw {
        root_task_len: 0,
        ..Raw::default()
    };
    assert_eq!(zero.parse(), Err(BootImageError::RootTaskLength));
    let overflow = Raw {
        root_task_len: u64::MAX,
        ..Raw::default()
    };
    assert_eq!(overflow.parse(), Err(BootImageError::RootTaskLength));
    let beyond = Raw {
        root_task_offset: PAGE_SIZE,
        root_task_len: IMAGE_LEN,
        archive_len: 0,
        ..Raw::default()
    };
    assert_eq!(beyond.parse(), Err(BootImageError::RootTaskLength));
}

#[test]
fn archive_offset_unaligned_inside_the_header_or_beyond_the_image_is_rejected() {
    let unaligned = Raw {
        archive_offset: 4 * PAGE_SIZE + 8,
        ..Raw::default()
    };
    assert_eq!(unaligned.parse(), Err(BootImageError::ArchiveOffset));
    let inside_header = Raw {
        archive_offset: 0,
        archive_len: 0,
        ..Raw::default()
    };
    assert_eq!(inside_header.parse(), Err(BootImageError::ArchiveOffset));
    let beyond = Raw {
        archive_offset: IMAGE_LEN + PAGE_SIZE,
        archive_len: 0,
        ..Raw::default()
    };
    assert_eq!(beyond.parse(), Err(BootImageError::ArchiveOffset));
}

#[test]
fn archive_length_overflowing_or_beyond_the_image_is_rejected() {
    let overflow = Raw {
        archive_len: u64::MAX,
        ..Raw::default()
    };
    assert_eq!(overflow.parse(), Err(BootImageError::ArchiveLength));
    let beyond = Raw {
        archive_offset: 4 * PAGE_SIZE,
        archive_len: IMAGE_LEN,
        ..Raw::default()
    };
    assert_eq!(beyond.parse(), Err(BootImageError::ArchiveLength));
}

#[test]
fn archive_of_length_zero_is_allowed_at_every_offset_behind_the_header() {
    for offset in [PAGE_SIZE, 4 * PAGE_SIZE, IMAGE_LEN] {
        let raw = Raw {
            archive_offset: offset,
            archive_len: 0,
            ..Raw::default()
        };
        assert_eq!(raw.parse().map(|header| header.archive_len), Ok(0));
    }
}

#[test]
fn archive_overlapping_the_root_task_at_either_end_is_rejected() {
    let root_offset = 4 * PAGE_SIZE;
    let root_len = 4 * PAGE_SIZE;
    let base = Raw {
        root_task_offset: root_offset,
        root_task_len: root_len,
        ..Raw::default()
    };
    let at_the_front = Raw {
        archive_offset: root_offset - PAGE_SIZE,
        archive_len: 2 * PAGE_SIZE,
        ..base
    };
    assert_eq!(at_the_front.parse(), Err(BootImageError::Overlap));
    let at_the_back = Raw {
        archive_offset: root_offset + root_len - PAGE_SIZE,
        archive_len: 2 * PAGE_SIZE,
        ..base
    };
    assert_eq!(at_the_back.parse(), Err(BootImageError::Overlap));
    let inside = Raw {
        archive_offset: root_offset + PAGE_SIZE,
        archive_len: PAGE_SIZE,
        ..base
    };
    assert_eq!(inside.parse(), Err(BootImageError::Overlap));
    let behind = Raw {
        archive_offset: root_offset + root_len,
        archive_len: PAGE_SIZE,
        ..base
    };
    assert!(behind.parse().is_ok());
}

#[test]
fn non_zero_flags_are_rejected() {
    for flags in [1, 1 << 63, u64::MAX] {
        let raw = Raw {
            flags,
            ..Raw::default()
        };
        assert_eq!(raw.parse(), Err(BootImageError::Flags(flags)));
    }
}

#[test]
fn reserve_size_unaligned_or_not_below_the_memory_is_rejected() {
    let unaligned = Raw {
        kernel_reserve_size: PAGE_SIZE + 1,
        ..Raw::default()
    };
    assert_eq!(
        unaligned.parse(),
        Err(BootImageError::ReserveSize(PAGE_SIZE + 1))
    );
    let too_large = Raw {
        kernel_reserve_size: RAM,
        ..Raw::default()
    };
    assert_eq!(too_large.parse(), Err(BootImageError::ReserveSize(RAM)));
    let fits = Raw {
        kernel_reserve_size: RAM - PAGE_SIZE,
        ..Raw::default()
    };
    assert_eq!(
        fits.parse().map(|header| header.kernel_reserve_size),
        Ok(RAM - PAGE_SIZE)
    );
    let default = Raw {
        kernel_reserve_size: 0,
        ..Raw::default()
    };
    assert!(default.parse().is_ok());
}

#[test]
fn an_image_of_exactly_the_header_size_holds_no_root_task() {
    let raw = Raw {
        root_task_offset: 0,
        root_task_len: 0,
        archive_offset: 0,
        archive_len: 0,
        ..Raw::default()
    };
    assert_eq!(
        raw.parse_in(u64::try_from(BOOT_IMAGE_HEADER_LEN).unwrap()),
        Err(BootImageError::RootTaskOffset)
    );
}

#[test]
fn serialized_headers_parse_back_unchanged() {
    check(
        "boot_image_round_trip",
        &any_boot_image_header(),
        |header| {
            let image_len = image_len_for(header);
            match BootImageHeader::parse(&header.to_bytes(), image_len, RAM) {
                Ok(parsed) if parsed == *header => Ok(()),
                Ok(parsed) => Err(format!("round trip changed the header: {parsed:?}")),
                Err(error) => Err(format!("valid header rejected: {error}")),
            }
        },
    );
}

#[test]
fn parsing_near_valid_bytes_never_panics() {
    check("boot_image_bytes", &any_boot_image_bytes(), |bytes| {
        let _ = BootImageHeader::parse(bytes, IMAGE_LEN, RAM);
        Ok(())
    });
}

#[test]
fn errors_render_a_message() {
    let errors = [
        BootImageError::TooShort,
        BootImageError::BadMagic,
        BootImageError::UnsupportedVersion(7),
        BootImageError::HeaderLength(3),
        BootImageError::RootTaskOffset,
        BootImageError::RootTaskLength,
        BootImageError::ArchiveOffset,
        BootImageError::ArchiveLength,
        BootImageError::Overlap,
        BootImageError::Flags(9),
        BootImageError::ReserveSize(11),
    ];
    for error in errors {
        assert!(!format!("{error}").is_empty());
    }
}
