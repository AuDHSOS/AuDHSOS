// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

#![expect(
    clippy::arithmetic_side_effects,
    reason = "Bounded fixture construction and test assertions"
)]

use crate::{Font, FontCollection, FontError, MAX_FACES, MAX_TABLES, OutlineKind, read};

fn put16(bytes: &mut [u8], at: usize, value: u16) {
    bytes[at..at + 2].copy_from_slice(&value.to_be_bytes());
}

fn put32(bytes: &mut [u8], at: usize, value: u32) {
    bytes[at..at + 4].copy_from_slice(&value.to_be_bytes());
}

fn number(value: usize) -> u32 {
    u32::try_from(value).expect("small fixture")
}

fn directory(bytes: &mut [u8], at: usize, kind: u32, records: &[([u8; 4], u32, u32)]) {
    put32(bytes, at, kind);
    put16(
        bytes,
        at + 4,
        u16::try_from(records.len()).expect("small directory"),
    );
    // Deliberately hostile search hints: readers must ignore these.
    bytes[at + 6..at + 12].fill(0xff);
    for (i, (tag, start, len)) in records.iter().enumerate() {
        let at = at + 12 + i * 16;
        bytes[at..at + 4].copy_from_slice(tag);
        put32(bytes, at + 4, 0x1234_5678);
        put32(bytes, at + 8, *start);
        put32(bytes, at + 12, *len);
    }
}

fn single() -> Vec<u8> {
    let mut bytes = vec![0; 51];
    directory(
        &mut bytes,
        0,
        0x0001_0000,
        &[(*b"TEST", 44, 4), (*b"zzzz", 48, 3)],
    );
    bytes[44..].copy_from_slice(&[1, 2, 3, 4, 5, 6, 7]);
    bytes
}

fn collection(version: u32, signature: bool) -> Vec<u8> {
    // v1 header ends at 20, v2 at 32; directories at 32 and 60.
    let mut bytes = vec![0; if signature { 100 } else { 92 }];
    bytes[..4].copy_from_slice(b"ttcf");
    put32(&mut bytes, 4, version);
    put32(&mut bytes, 8, 2);
    put32(&mut bytes, 12, 32);
    put32(&mut bytes, 16, 60);
    directory(&mut bytes, 32, 0x0001_0000, &[(*b"TEST", 88, 4)]);
    directory(&mut bytes, 60, 0x4f54_544f, &[(*b"TEST", 88, 4)]);
    bytes[88..92].copy_from_slice(&[7, 8, 9, 10]);
    if signature {
        put32(&mut bytes, 20, 0x4453_4947);
        put32(&mut bytes, 24, 8);
        put32(&mut bytes, 28, 92);
    }
    bytes
}

fn error(bytes: &[u8], expected: FontError) {
    assert_eq!(FontCollection::parse(bytes).err(), Some(expected));
    assert_eq!(Font::parse(bytes).err(), Some(expected));
}

#[test]
fn sfnt_both_kinds_preserve_exact_payloads() -> Result<(), FontError> {
    for (signature, expected) in [
        (0x0001_0000, OutlineKind::TrueType),
        (0x4f54_544f, OutlineKind::PostScript),
    ] {
        let mut bytes = single();
        put32(&mut bytes, 0, signature);
        let font = Font::parse(&bytes)?;
        assert_eq!(font.outline_kind(), expected);
        assert_eq!(font.table_count(), 2);
        let table = font.table(*b"zzzz").expect("second table");
        assert_eq!(
            (table.offset, table.checksum, table.data),
            (48, 0x1234_5678, &[5, 6, 7][..])
        );
        assert_eq!(
            font.tables().map(|t| t.tag).collect::<Vec<_>>(),
            [*b"TEST", *b"zzzz"]
        );
        assert!(font.table(*b"AAAA").is_none());
        assert!(font.table(*b"UVWX").is_none());
        assert!(font.table([0xff; 4]).is_none());
    }
    Ok(())
}

#[test]
fn collection_versions_allow_exact_sharing() -> Result<(), FontError> {
    for (version, signature) in [
        (0x0001_0000, false),
        (0x0002_0000, false),
        (0x0002_0000, true),
    ] {
        let bytes = collection(version, signature);
        let file = FontCollection::parse(&bytes)?;
        assert_eq!(file.face_count(), 2);
        assert_eq!(file.font(0)?.table(*b"TEST"), file.font(1)?.table(*b"TEST"));
        assert_eq!(file.font(1)?.outline_kind(), OutlineKind::PostScript);
        assert_eq!(file.font(2).err(), Some(FontError::FaceIndex));
        assert_eq!(file.font(u32::MAX).err(), Some(FontError::FaceIndex));
        assert_eq!(Font::parse(&bytes)?.outline_kind(), OutlineKind::TrueType);
    }
    let bytes = single();
    let file = FontCollection::parse(&bytes)?;
    assert_eq!(file.face_count(), 1);
    assert_eq!(file.font(1).err(), Some(FontError::FaceIndex));
    Ok(())
}

#[test]
fn sfnt_every_truncated_prefix_returns_error() {
    for bytes in [
        single(),
        collection(0x0001_0000, false),
        collection(0x0002_0000, false),
        collection(0x0002_0000, true),
    ] {
        for len in 0..bytes.len() {
            assert!(
                FontCollection::parse(&bytes[..len]).is_err(),
                "accepted prefix {len}"
            );
        }
    }
}

#[test]
fn sfnt_unsupported_signatures_and_collection_versions_return_errors() {
    for signature in [
        0,
        0x7472_7565,
        0x7479_7031,
        0x774f_4646,
        0x774f_4632,
        u32::MAX,
    ] {
        let mut bytes = single();
        put32(&mut bytes, 0, signature);
        error(&bytes, FontError::UnsupportedFormat);
    }
    for version in [0, 0x0001_0001, 0x0002_0001, 0x0003_0000, u32::MAX] {
        error(&collection(version, false), FontError::UnsupportedFormat);
    }
}

#[test]
fn sfnt_invalid_tags_and_order_return_errors() {
    for tag in [
        *b"    ",
        *b" TES",
        *b"T ST",
        [0, 1, 2, 3],
        [0x7f; 4],
        [0xff; 4],
    ] {
        let mut bytes = single();
        bytes[12..16].copy_from_slice(&tag);
        error(&bytes, FontError::InvalidTag);
    }
    for tag in [*b"TEST", *b"AAAA"] {
        let mut bytes = single();
        bytes[28..32].copy_from_slice(&tag);
        error(&bytes, FontError::TableOrder);
    }
    let mut bytes = single();
    bytes[12..16].copy_from_slice(b"CFF ");
    assert!(Font::parse(&bytes).is_ok());
}

#[test]
fn sfnt_payload_offsets_and_lengths_are_checked() {
    for (start, len, expected) in [
        (44, 8, FontError::Truncated),
        (52, 0, FontError::Truncated),
        (u32::MAX, 2, FontError::Overflow),
        (44, u32::MAX, FontError::Overflow),
        (45, 3, FontError::Misaligned),
        (0, 4, FontError::Overlap),
        (12, 16, FontError::Overlap),
        (40, 8, FontError::Overlap),
        (44, 7, FontError::Overlap),
        (48, 3, FontError::Overlap),
        (12, 0, FontError::Overlap),
        (0, 0, FontError::Overlap),
    ] {
        let mut bytes = single();
        put32(&mut bytes, 20, start);
        put32(&mut bytes, 24, len);
        error(&bytes, expected);
    }
}

#[test]
fn sfnt_empty_tables_at_eof_and_adjacent_ranges_are_valid() -> Result<(), FontError> {
    let mut bytes = single();
    bytes.push(0);
    put32(&mut bytes, 20, 52);
    put32(&mut bytes, 24, 0);
    assert!(
        Font::parse(&bytes)?
            .table(*b"TEST")
            .expect("empty table")
            .data
            .is_empty()
    );
    // Table payload order need not match tag order.
    put32(&mut bytes, 20, 48);
    put32(&mut bytes, 24, 3);
    put32(&mut bytes, 36, 44);
    put32(&mut bytes, 40, 4);
    assert_eq!(
        Font::parse(&bytes)?.table(*b"TEST").expect("table").data,
        [5, 6, 7]
    );
    Ok(())
}

#[test]
fn collection_self_reference_cycles_and_directory_aliases_are_rejected() {
    for (field, start, expected) in [
        (12, 0, FontError::UnsupportedFormat),
        (16, 32, FontError::Overlap),
        (12, 33, FontError::Misaligned),
        (12, 0xffff_fffc, FontError::Truncated),
        (12, 84, FontError::Truncated),
    ] {
        let mut bytes = collection(0x0001_0000, false);
        put32(&mut bytes, field, start);
        error(&bytes, expected);
    }
    // A nested TTC pointing back at the outer TTC is never recursively followed.
    let mut bytes = collection(0x0001_0000, false);
    bytes[32..36].copy_from_slice(b"ttcf");
    put32(&mut bytes, 36, 0x0001_0000);
    put32(&mut bytes, 40, 1);
    put32(&mut bytes, 44, 0);
    error(&bytes, FontError::UnsupportedFormat);
}

#[test]
fn collection_metadata_and_partial_sharing_are_rejected() {
    for (field, value) in [(52, 0), (52, 12), (52, 32), (52, 60), (80, 32), (48, 9)] {
        let mut bytes = collection(0x0001_0000, false);
        put32(&mut bytes, field, value);
        error(&bytes, FontError::Overlap);
    }
    let mut bytes = collection(0x0001_0000, false);
    put32(&mut bytes, 84, 3);
    error(&bytes, FontError::Overlap);
    bytes[72..76].copy_from_slice(b"ZZZZ");
    put32(&mut bytes, 84, 4);
    error(&bytes, FontError::Overlap);
    // A valid-looking directory can overlap the TTC header.
    let mut bytes = collection(0x0001_0000, false);
    put32(&mut bytes, 12, 20);
    directory(&mut bytes, 20, 0x0001_0000, &[(*b"TEST", 88, 4)]);
    // Raise the offset array to include the start of that directory.
    put32(&mut bytes, 8, 3);
    error(&bytes, FontError::Overlap);
}

#[test]
fn collection_overlapping_directories_are_rejected() {
    let mut bytes = collection(0x0001_0000, false);
    // Put a second directory inside the first directory's record array.
    // First record remains a legal tag/checksum/offset/length.
    bytes[44..48].copy_from_slice(b"OTTO");
    put32(&mut bytes, 48, 0x0001_0000);
    put32(&mut bytes, 16, 44);
    error(&bytes, FontError::Overlap);
}

#[test]
fn collection_dsig_fields_and_ranges_are_checked() {
    for (tag, len, start, expected) in [
        (0, 8, 92, FontError::InvalidSignature),
        (0x4453_4947, 0, 92, FontError::InvalidSignature),
        (1, 8, 92, FontError::InvalidSignature),
        (0, 0, 92, FontError::InvalidSignature),
        (0x4453_4947, 4, 92, FontError::InvalidSignature),
        (0x4453_4947, 100, 0, FontError::InvalidSignature),
        (0x4453_4947, 68, 32, FontError::Overlap),
        (0x4453_4947, 12, 88, FontError::Overlap),
        (0x4453_4947, 7, 93, FontError::Misaligned),
        (0x4453_4947, 9, 92, FontError::Truncated),
        (0x4453_4947, u32::MAX, 92, FontError::Overflow),
    ] {
        let mut bytes = collection(0x0002_0000, true);
        put32(&mut bytes, 20, tag);
        put32(&mut bytes, 24, len);
        put32(&mut bytes, 28, start);
        error(&bytes, expected);
    }
}

#[test]
fn sfnt_zero_and_excessive_counts_are_rejected() {
    for (count, expected) in [
        (0, FontError::EmptyDirectory),
        (MAX_TABLES + 1, FontError::LimitExceeded),
        (u16::MAX, FontError::LimitExceeded),
    ] {
        let mut bytes = single();
        put16(&mut bytes, 4, count);
        error(&bytes, expected);
    }
    for (count, expected) in [
        (0, FontError::EmptyDirectory),
        (MAX_FACES + 1, FontError::LimitExceeded),
        (u32::MAX, FontError::LimitExceeded),
    ] {
        let mut bytes = collection(0x0001_0000, false);
        put32(&mut bytes, 8, count);
        error(&bytes, expected);
    }
}

fn sized_collection(faces: usize, tables: usize) -> Vec<u8> {
    let header = 12 + faces * 4;
    let directory_size = 12 + tables * 16;
    let end = header + faces * directory_size;
    let mut bytes = vec![0; end];
    bytes[..4].copy_from_slice(b"ttcf");
    put32(&mut bytes, 4, 0x0001_0000);
    put32(&mut bytes, 8, number(faces));
    let records: Vec<_> = (0..tables)
        .map(|i| {
            let tag = [
                b'A',
                b'A',
                b'A' + u8::try_from(i / 26).expect("small tag"),
                b'A' + u8::try_from(i % 26).expect("small tag"),
            ];
            (tag, number(end), 0)
        })
        .collect();
    for i in 0..faces {
        let start = header + i * directory_size;
        put32(&mut bytes, 12 + 4 * i, number(start));
        directory(&mut bytes, start, 0x0001_0000, &records);
    }
    bytes
}

#[test]
fn collection_work_limits_accept_equality_reject_excess() -> Result<(), FontError> {
    assert_eq!(
        FontCollection::parse(&sized_collection(64, 1))?.face_count(),
        64
    );
    assert_eq!(Font::parse(&sized_collection(1, 256))?.table_count(), 256);
    assert_eq!(
        FontCollection::parse(&sized_collection(16, 256))?.face_count(),
        16
    );
    error(&sized_collection(17, 256), FontError::LimitExceeded);
    Ok(())
}

fn serialize(bytes: &[u8]) -> Result<Vec<u8>, FontError> {
    let file = FontCollection::parse(bytes)?;
    let mut out = Vec::new();
    out.extend_from_slice(&file.face_count().to_be_bytes());
    for i in 0..file.face_count() {
        let font = file.font(i)?;
        out.push(match font.outline_kind() {
            OutlineKind::TrueType => 0,
            OutlineKind::PostScript => 1,
        });
        out.extend_from_slice(&font.table_count().to_be_bytes());
        for table in font.tables() {
            out.extend_from_slice(&table.tag);
            out.extend_from_slice(&table.checksum.to_be_bytes());
            out.extend_from_slice(&table.offset.to_be_bytes());
            out.extend_from_slice(&number(table.data.len()).to_be_bytes());
            out.extend_from_slice(table.data);
        }
    }
    Ok(out)
}

#[test]
fn sfnt_separate_allocations_produce_identical_bytes() -> Result<(), FontError> {
    for first in [
        single(),
        collection(0x0001_0000, false),
        collection(0x0002_0000, true),
    ] {
        let second = first.clone();
        assert_ne!(first.as_ptr(), second.as_ptr());
        assert_eq!(serialize(&first)?, serialize(&second)?);
    }
    Ok(())
}

#[test]
fn sfnt_host_file_reads_preserve_numbers() -> Result<(), Box<dyn std::error::Error>> {
    let bytes = std::fs::read(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/src/tests/fixtures/envelope.sfnt"
    ))?;
    let font = Font::parse(&bytes)?;
    assert_eq!(font.table_count(), 2);
    assert_eq!(
        font.table(*b"TEST").expect("fixture table").data,
        [1, 2, 3, 4]
    );
    assert_eq!(serialize(&bytes)?, serialize(&single())?);
    Ok(())
}

#[test]
fn sfnt_single_byte_mutations_and_arbitrary_inputs_never_panic() {
    for seed in [single(), collection(0x0002_0000, true)] {
        for i in 0..seed.len() {
            for byte in 0..=u8::MAX {
                let mut bytes = seed.clone();
                bytes[i] = byte;
                if let Ok(file) = FontCollection::parse(&bytes) {
                    for index in 0..file.face_count() {
                        let font = file.font(index).expect("validated face");
                        assert_eq!(font.tables().count(), usize::from(font.table_count()));
                        for table in font.tables() {
                            let start = usize::try_from(table.offset).expect("bounded offset");
                            assert_eq!(table.data, &bytes[start..start + table.data.len()]);
                            assert_eq!(font.table(table.tag), Some(table));
                        }
                    }
                }
            }
        }
    }
    let mut state = 1u64;
    for len in 0..512 {
        let bytes: Vec<_> = (0..len)
            .map(|_| {
                state = state
                    .wrapping_mul(6_364_136_223_846_793_005)
                    .wrapping_add(1);
                state.to_be_bytes()[0]
            })
            .collect();
        let _ = FontCollection::parse(&bytes);
    }
}

#[test]
fn checked_reads_reject_arithmetic_overflow() {
    assert_eq!(read::add(usize::MAX, 1), Err(FontError::Overflow));
    assert_eq!(read::mul(usize::MAX, 2), Err(FontError::Overflow));
    assert_eq!(read::bytes(&[], usize::MAX, 1), Err(FontError::Overflow));
    assert_eq!(read::u16(&[1], 0), Err(FontError::Truncated));
    assert_eq!(read::offset(0), Ok(0));
}

#[test]
fn font_errors_have_distinct_stable_descriptions() {
    let errors = [
        FontError::MissingTable,
        FontError::InvalidTable,
        FontError::GlyphIndex,
        FontError::Truncated,
        FontError::Overflow,
        FontError::UnsupportedFormat,
        FontError::EmptyDirectory,
        FontError::LimitExceeded,
        FontError::Misaligned,
        FontError::InvalidTag,
        FontError::TableOrder,
        FontError::Overlap,
        FontError::InvalidSignature,
        FontError::FaceIndex,
    ];
    let descriptions: std::collections::BTreeSet<_> =
        errors.iter().map(ToString::to_string).collect();
    assert_eq!(descriptions.len(), errors.len());
    assert!(descriptions.iter().all(|s| !s.is_empty()));
}
