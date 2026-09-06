// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

//! Tests of `crate::tar`, against the tar items of the edge-case catalog
//! 6.6.13.

use test_support::property::check;

use crate::strategies::{any_archive, write};
use crate::tar::{
    Archive, BLOCK, Builder, CHECKSUM, Kind, MAGIC, NAME, PREFIX, SIZE, TarError, WriteError,
    archive_len,
};

/// The bytes of an archive holding `files`.
fn archive(files: &[(&[u8], &[u8])]) -> Vec<u8> {
    let mut bytes = vec![0u8; archive_len(files)];
    let written = {
        let mut builder = Builder::new(&mut bytes);
        for (name, data) in files {
            builder.file(name, data).unwrap();
        }
        builder.finish().unwrap()
    };
    bytes.truncate(written);
    bytes
}

/// One entry, taken apart into values a test can compare.
type Read = (Vec<u8>, Kind, Vec<u8>);

/// Every entry of `bytes`, or the first error.
fn entries(bytes: &[u8]) -> Result<Vec<Read>, TarError> {
    Archive::new(bytes)
        .entries()
        .map(|entry| {
            entry.map(|entry| {
                (
                    entry.path.as_bytes().to_vec(),
                    entry.kind,
                    entry.data.to_vec(),
                )
            })
        })
        .collect()
}

/// Overwrites the byte at `offset` of the first header.
fn poke(bytes: &mut [u8], offset: usize, value: u8) {
    *bytes.get_mut(offset).unwrap() = value;
}

#[test]
fn an_empty_archive_holds_no_entry() {
    let bytes = archive(&[]);
    assert_eq!(bytes.len(), BLOCK.wrapping_mul(2));
    assert_eq!(entries(&bytes).unwrap(), Vec::new());
}

#[test]
fn one_file_comes_back_with_its_name_and_its_bytes() {
    let bytes = archive(&[(b"app-hello", b"hello from userland\n")]);
    let read = entries(&bytes).unwrap();
    assert_eq!(read.len(), 1);
    let (name, kind, data) = read.first().unwrap();
    assert_eq!(name.as_slice(), b"app-hello");
    assert_eq!(*kind, Kind::File);
    assert_eq!(data.as_slice(), b"hello from userland\n");
}

#[test]
fn several_files_come_back_in_the_order_they_were_written() {
    let bytes = archive(&[
        (b"server-memory", b"one"),
        (b"server-name", b"two"),
        (b"app-hello", b"three"),
    ]);
    let read = entries(&bytes).unwrap();
    let names: Vec<&[u8]> = read.iter().map(|(name, _, _)| name.as_slice()).collect();
    assert_eq!(
        names,
        vec![
            b"server-memory".as_slice(),
            b"server-name".as_slice(),
            b"app-hello".as_slice()
        ]
    );
}

#[test]
fn a_file_of_no_bytes_is_an_entry_with_no_bytes() {
    let bytes = archive(&[(b"empty", b"")]);
    let read = entries(&bytes).unwrap();
    let (_, _, data) = read.first().unwrap();
    assert!(data.is_empty());
    // Header and the two end blocks, and no content block.
    assert_eq!(bytes.len(), BLOCK.wrapping_mul(3));
}

#[test]
fn a_size_that_is_no_multiple_of_the_block_is_padded_and_read_back_exactly() {
    let content = vec![0x41u8; 513];
    let bytes = archive(&[(b"odd", &content)]);
    assert_eq!(bytes.len(), BLOCK.wrapping_mul(5));
    let read = entries(&bytes).unwrap();
    let (_, _, data) = read.first().unwrap();
    assert_eq!(data.len(), 513);
    assert!(data.iter().all(|byte| *byte == 0x41));
}

#[test]
fn a_name_that_needs_the_prefix_field_comes_back_joined() {
    let long = b"boot/programs/and/a/directory/deep/enough/that/the/name/field/alone/cannot/hold/it/server-console";
    assert!(long.len() <= 100);
    let deeper: Vec<u8> = [b"a/".repeat(20).as_slice(), long].concat();
    assert!(deeper.len() > 100);
    let bytes = archive(&[(&deeper, b"x")]);
    let read = entries(&bytes).unwrap();
    let (name, _, _) = read.first().unwrap();
    assert_eq!(name.as_slice(), deeper.as_slice());
}

#[test]
fn a_directory_entry_is_read_as_one() {
    let mut bytes = vec![0u8; archive_len(&[(b"boot", b"")])];
    let written = {
        let mut builder = Builder::new(&mut bytes);
        builder.entry(b"boot", Kind::Directory, b"").unwrap();
        builder.finish().unwrap()
    };
    bytes.truncate(written);
    let read = entries(&bytes).unwrap();
    let (name, kind, _) = read.first().unwrap();
    assert_eq!(name.as_slice(), b"boot");
    assert_eq!(*kind, Kind::Directory);
}

#[test]
fn a_type_flag_this_system_has_no_use_for_is_handed_out_as_it_stands() {
    let mut bytes = vec![0u8; archive_len(&[(b"link", b"")])];
    let written = {
        let mut builder = Builder::new(&mut bytes);
        builder.entry(b"link", Kind::Other(b'2'), b"").unwrap();
        builder.finish().unwrap()
    };
    bytes.truncate(written);
    let read = entries(&bytes).unwrap();
    let (_, kind, _) = read.first().unwrap();
    assert_eq!(*kind, Kind::Other(b'2'));
}

#[test]
fn a_zero_type_flag_is_a_regular_file() {
    let mut bytes = archive(&[(b"old", b"x")]);
    poke(&mut bytes, 156, 0);
    fix_checksum(&mut bytes);
    let read = entries(&bytes).unwrap();
    let (_, kind, _) = read.first().unwrap();
    assert_eq!(*kind, Kind::File);
}

#[test]
fn the_walk_ends_at_the_end_marker_whatever_follows_it() {
    let mut bytes = archive(&[(b"one", b"x")]);
    bytes.extend_from_slice(&[0x5A; 1024]);
    let read = entries(&bytes).unwrap();
    assert_eq!(read.len(), 1);
}

#[test]
fn an_archive_that_stops_inside_a_header_ends_there() {
    let bytes = archive(&[(b"one", b"x")]);
    let cut = bytes
        .get(..BLOCK.wrapping_mul(2).wrapping_add(100))
        .unwrap()
        .to_vec();
    // The first entry is whole; what follows is less than a block.
    let read = entries(&cut).unwrap();
    assert_eq!(read.len(), 1);
}

#[test]
fn an_archive_that_stops_inside_a_file_is_truncated() {
    let content = vec![0x41u8; 1024];
    let bytes = archive(&[(b"big", &content)]);
    let cut = bytes.get(..BLOCK.wrapping_mul(2)).unwrap().to_vec();
    assert_eq!(
        entries(&cut).unwrap_err(),
        TarError::Truncated { at: BLOCK }
    );
}

#[test]
fn a_header_whose_magic_is_not_ustar_is_refused() {
    let mut bytes = archive(&[(b"one", b"x")]);
    poke(&mut bytes, MAGIC, b'x');
    assert_eq!(entries(&bytes).unwrap_err(), TarError::BadMagic);
}

#[test]
fn a_header_whose_checksum_does_not_match_is_refused() {
    let mut bytes = archive(&[(b"one", b"x")]);
    // One byte of the name, which the checksum covers.
    poke(&mut bytes, NAME, b'X');
    assert!(matches!(
        entries(&bytes).unwrap_err(),
        TarError::BadChecksum { .. }
    ));
}

#[test]
fn a_size_field_that_is_not_octal_is_refused() {
    let mut bytes = archive(&[(b"one", b"x")]);
    poke(&mut bytes, SIZE, b'9');
    fix_checksum(&mut bytes);
    assert_eq!(
        entries(&bytes).unwrap_err(),
        TarError::BadOctal { at: SIZE }
    );
}

#[test]
fn a_checksum_field_that_is_not_octal_is_refused() {
    let mut bytes = archive(&[(b"one", b"x")]);
    poke(&mut bytes, CHECKSUM, b'z');
    assert_eq!(
        entries(&bytes).unwrap_err(),
        TarError::BadOctal { at: CHECKSUM }
    );
}

#[test]
fn an_absolute_name_is_refused() {
    let mut bytes = archive(&[(b"one", b"x")]);
    poke(&mut bytes, NAME, b'/');
    fix_checksum(&mut bytes);
    assert_eq!(entries(&bytes).unwrap_err(), TarError::AbsolutePath);
}

#[test]
fn a_name_with_a_parent_component_is_refused() {
    let bytes = archive(&[(b"../escape", b"x")]);
    assert_eq!(entries(&bytes).unwrap_err(), TarError::ParentComponent);
    let bytes = archive(&[(b"boot/../escape", b"x")]);
    assert_eq!(entries(&bytes).unwrap_err(), TarError::ParentComponent);
    // A name that merely begins with two dots is not one.
    let bytes = archive(&[(b"..hidden", b"x")]);
    assert_eq!(entries(&bytes).unwrap().len(), 1);
}

#[test]
fn a_header_without_a_name_is_refused() {
    let mut bytes = archive(&[(b"one", b"x")]);
    for offset in NAME..NAME.wrapping_add(3) {
        poke(&mut bytes, offset, 0);
    }
    fix_checksum(&mut bytes);
    assert_eq!(entries(&bytes).unwrap_err(), TarError::EmptyName);
}

#[test]
fn a_name_with_bytes_behind_its_zero_is_refused() {
    let mut bytes = archive(&[(b"one", b"x")]);
    poke(&mut bytes, NAME.wrapping_add(5), b'x');
    fix_checksum(&mut bytes);
    assert_eq!(
        entries(&bytes).unwrap_err(),
        TarError::NameNotTerminated { at: NAME }
    );
}

#[test]
fn a_prefix_with_bytes_behind_its_zero_is_refused() {
    let mut bytes = archive(&[(b"one", b"x")]);
    poke(&mut bytes, PREFIX.wrapping_add(5), b'x');
    fix_checksum(&mut bytes);
    assert_eq!(
        entries(&bytes).unwrap_err(),
        TarError::NameNotTerminated { at: PREFIX }
    );
}

#[test]
fn a_size_that_does_not_fit_the_archive_is_truncated() {
    let mut bytes = archive(&[(b"one", b"x")]);
    // Eight gibibytes, in octal, which no archive of this system holds.
    let field = b"77777777777\0";
    for (offset, byte) in field.iter().enumerate() {
        poke(&mut bytes, SIZE.wrapping_add(offset), *byte);
    }
    fix_checksum(&mut bytes);
    assert!(matches!(
        entries(&bytes).unwrap_err(),
        TarError::Truncated { .. }
    ));
}

#[test]
fn a_name_is_found_and_a_name_that_is_not_there_is_not() {
    let bytes = archive(&[(b"server-name", b"one"), (b"app-hello", b"two")]);
    let found = Archive::new(&bytes).find(b"app-hello").unwrap().unwrap();
    assert_eq!(found.data, b"two");
    assert!(Archive::new(&bytes).find(b"missing").unwrap().is_none());
}

#[test]
fn a_search_through_a_broken_archive_reports_the_break() {
    let mut bytes = archive(&[(b"one", b"x")]);
    poke(&mut bytes, MAGIC, b'x');
    assert_eq!(
        Archive::new(&bytes).find(b"one").unwrap_err(),
        TarError::BadMagic
    );
}

#[test]
fn the_archive_hands_back_the_bytes_it_was_built_from() {
    let bytes = archive(&[]);
    assert_eq!(Archive::new(&bytes).bytes(), bytes.as_slice());
}

#[test]
fn a_buffer_too_small_for_the_archive_is_refused() {
    // Room for one header and one content block, and nothing else.
    let mut small = [0u8; BLOCK * 2];
    let mut builder = Builder::new(&mut small);
    assert!(builder.is_empty());
    builder.file(b"one", b"x").unwrap();
    assert_eq!(builder.len(), BLOCK.wrapping_mul(2));
    assert_eq!(builder.file(b"two", b"y"), Err(WriteError::Full));
    assert_eq!(builder.finish(), Err(WriteError::Full));
}

#[test]
fn a_name_that_fits_no_split_is_refused() {
    let long = vec![b'a'; 300];
    let mut bytes = [0u8; BLOCK * 4];
    let mut builder = Builder::new(&mut bytes);
    assert_eq!(builder.file(&long, b""), Err(WriteError::NameTooLong(300)));
}

#[test]
fn every_error_renders_a_message() {
    let cases = [
        TarError::Truncated { at: 512 },
        TarError::BadMagic,
        TarError::BadChecksum {
            found: 1,
            computed: 2,
        },
        TarError::BadOctal { at: 124 },
        TarError::AbsolutePath,
        TarError::ParentComponent,
        TarError::EmptyName,
        TarError::NameNotTerminated { at: 0 },
    ];
    for case in cases {
        assert!(!format!("{case}").is_empty(), "{case:?} has no message");
    }
    let writes = [WriteError::Full, WriteError::NameTooLong(300)];
    for case in writes {
        assert!(!format!("{case}").is_empty(), "{case:?} has no message");
    }
}

#[test]
fn what_the_writer_wrote_the_reader_reads() {
    check("tar round trip", &any_archive(), |bytes| {
        match entries(bytes) {
            Ok(_) => Ok(()),
            Err(error) => Err(format!("{error}")),
        }
    });
}

#[test]
fn the_names_come_back_out_of_a_generated_archive() {
    let files = vec![
        (b"server-memory".to_vec(), vec![1u8, 2, 3]),
        (b"app-hello".to_vec(), Vec::new()),
    ];
    let bytes = write(&files);
    let read = entries(&bytes).unwrap();
    assert_eq!(read.len(), 2);
    let (name, _, data) = read.first().unwrap();
    assert_eq!(name.as_slice(), b"server-memory");
    assert_eq!(data.as_slice(), &[1, 2, 3]);
}

/// Recomputes the checksum of the first header after a test has changed a
/// byte the checksum covers, so that the test sees the check it is after
/// and not the checksum check in front of it.
fn fix_checksum(bytes: &mut [u8]) {
    for offset in CHECKSUM..CHECKSUM.wrapping_add(8) {
        *bytes.get_mut(offset).unwrap() = b' ';
    }
    let sum = bytes
        .get(..BLOCK)
        .unwrap()
        .iter()
        .fold(0u64, |total, byte| total.wrapping_add(u64::from(*byte)));
    let digits = format!("{sum:06o}\0 ");
    for (offset, byte) in digits.bytes().enumerate() {
        *bytes.get_mut(CHECKSUM.wrapping_add(offset)).unwrap() = byte;
    }
}

#[test]
fn a_path_says_how_long_it_is() {
    let bytes = archive(&[(b"server-console", b"")]);
    let entry = Archive::new(&bytes)
        .find(b"server-console")
        .unwrap()
        .unwrap();
    assert_eq!(entry.path.len(), 14);
    assert!(!entry.path.is_empty());
}

#[test]
fn a_size_field_of_nothing_but_padding_is_zero() {
    // Spaces and zeros, both of which older writers produced for a file of
    // no bytes, and in one field so that neither is read as a digit.
    for filler in [b' ', 0u8] {
        let mut bytes = archive(&[(b"one", b"")]);
        for offset in SIZE..SIZE.wrapping_add(12) {
            poke(&mut bytes, offset, filler);
        }
        fix_checksum(&mut bytes);
        let read = entries(&bytes).unwrap();
        let (_, _, data) = read.first().unwrap();
        assert!(data.is_empty(), "filler {filler}");
    }
}

#[test]
fn the_walk_stays_ended_once_it_has_ended() {
    let bytes = archive(&[(b"one", b"x")]);
    let mut walk = Archive::new(&bytes).entries();
    assert!(walk.next().is_some());
    assert!(walk.next().is_none());
    assert!(walk.next().is_none(), "a walk that ended hands out more");

    let mut bytes = archive(&[(b"one", b"x")]);
    poke(&mut bytes, MAGIC, b'x');
    let mut walk = Archive::new(&bytes).entries();
    assert!(walk.next().is_some_and(|entry| entry.is_err()));
    assert!(walk.next().is_none(), "a walk that failed hands out more");
}

#[test]
fn a_header_with_a_prefix_and_no_name_is_the_prefix_alone() {
    // The writer cannot build this: it puts a name in the name field and
    // uses the prefix only for what does not fit. A header of another
    // writer may all the same.
    let mut bytes = archive(&[(b"one", b"")]);
    for offset in NAME..NAME.wrapping_add(3) {
        poke(&mut bytes, offset, 0);
    }
    for (offset, byte) in b"boot".iter().enumerate() {
        poke(&mut bytes, PREFIX.wrapping_add(offset), *byte);
    }
    fix_checksum(&mut bytes);
    let read = entries(&bytes).unwrap();
    let (name, _, _) = read.first().unwrap();
    assert_eq!(name.as_slice(), b"boot");
}

#[test]
fn a_long_name_whose_only_slash_is_too_far_in_fits_no_split() {
    // The slash stands beyond the prefix field, so there is no cut that
    // leaves at most a hundred bytes for the name field.
    let name: Vec<u8> = [vec![b'a'; 200], vec![b'/'], vec![b'b'; 20]].concat();
    let mut bytes = [0u8; BLOCK * 4];
    let mut builder = Builder::new(&mut bytes);
    assert_eq!(
        builder.file(&name, b""),
        Err(WriteError::NameTooLong(name.len()))
    );
}

#[test]
fn a_size_field_that_begins_with_a_space_is_read_behind_it() {
    let mut bytes = archive(&[(b"one", b"x")]);
    poke(&mut bytes, SIZE, b' ');
    fix_checksum(&mut bytes);
    let read = entries(&bytes).unwrap();
    let (_, _, data) = read.first().unwrap();
    assert_eq!(data.as_slice(), b"x");
}

#[test]
fn a_name_that_is_exactly_the_width_of_the_name_field_needs_no_prefix() {
    let name = vec![b'n'; 100];
    let bytes = archive(&[(&name, b"x")]);
    let read = entries(&bytes).unwrap();
    let (read_name, _, _) = read.first().unwrap();
    assert_eq!(read_name.as_slice(), name.as_slice());
}

#[test]
fn a_name_of_the_full_width_of_both_fields_comes_back_whole() {
    let name: Vec<u8> = [vec![b'p'; 155], vec![b'/'], vec![b'n'; 100]].concat();
    assert_eq!(name.len(), 256);
    let bytes = archive(&[(&name, b"x")]);
    let read = entries(&bytes).unwrap();
    let (read_name, _, _) = read.first().unwrap();
    assert_eq!(read_name.as_slice(), name.as_slice());
}
