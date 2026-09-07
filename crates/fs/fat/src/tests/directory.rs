// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

//! Names in the 8.3 form, and the directories made of them.

use crate::device::{BlockDevice, SECTOR, put};
use crate::dir::{
    ATTR_ARCHIVE, ATTR_DIRECTORY, ATTR_LONG_NAME, ATTR_VOLUME_ID, DELETED, ENTRY_LEN,
};
use crate::error::Error;
use crate::fs::FileSystem;
use crate::name::Name;
use crate::tests::support::{moment, volume};

/// Writes `bytes` into slot `slot` of the cluster the root begins at,
/// which is how a test puts an entry there that no writer of this crate
/// would.
fn put_slot(fs: &mut FileSystem<crate::doubles::RamDisk>, slot: usize, bytes: &[u8]) {
    let sector = fs.geometry().cluster_sector(fs.root().cluster());
    let mut buffer = [0u8; SECTOR];
    fs.device().read(sector, &mut buffer).expect("read");
    put(&mut buffer, slot.saturating_mul(ENTRY_LEN), bytes);
    fs.device_mut().write(sector, &buffer).expect("write");
}

#[test]
fn a_name_with_and_without_an_extension_is_padded_to_eleven_bytes() {
    assert_eq!(
        Name::new("KERNEL.ELF").expect("name").as_bytes(),
        b"KERNEL  ELF"
    );
    assert_eq!(Name::new("EFI").expect("name").as_bytes(), b"EFI        ");
    assert_eq!(Name::new("A.B").expect("name").as_bytes(), b"A       B  ");
    assert_eq!(
        Name::new("BOOTX64.EFI").expect("name").as_bytes(),
        b"BOOTX64 EFI"
    );
}

#[test]
fn a_name_a_caller_writes_in_lower_case_is_upper_cased() {
    assert_eq!(
        Name::new("kernel.elf").expect("name").as_bytes(),
        b"KERNEL  ELF"
    );
}

#[test]
fn a_name_the_short_form_cannot_carry_is_refused() {
    assert_eq!(Name::new(""), Err(Error::Name));
    assert_eq!(Name::new("TOOLONGNAME.BIN"), Err(Error::Name));
    assert_eq!(Name::new("NAME.LONG"), Err(Error::Name));
    assert_eq!(Name::new("NAME."), Err(Error::Name));
    assert_eq!(Name::new("NA ME"), Err(Error::Name));
    assert_eq!(Name::new("NA+ME"), Err(Error::Name));
    assert_eq!(Name::new(".BIN"), Err(Error::Name));
    assert_eq!(Name::new("A.B.C"), Err(Error::Name));
    assert!(Name::new("A$%'-_@~`!(){}^#&".get(..8).expect("eight")).is_ok());
}

#[test]
fn an_entry_written_in_lower_case_is_not_a_name_this_crate_reads() {
    assert_eq!(Name::from_entry(*b"kernel  elf"), Err(Error::EntryName));
    assert_eq!(Name::from_entry(*b"           "), Err(Error::EntryName));
    assert_eq!(Name::from_entry(*b"NAME+   BIN"), Err(Error::EntryName));
    assert_eq!(
        Name::from_entry(*b"KERNEL  ELF").expect("name"),
        Name::new("KERNEL.ELF").expect("name")
    );
}

#[test]
fn a_name_writes_itself_back_the_way_it_was_given() {
    for text in ["KERNEL.ELF", "EFI", "BOOTX64.EFI", "A.B", "F001.BIN"] {
        let name = Name::new(text).expect("name");
        assert_eq!(std::format!("{name}"), text);
    }
    assert!(Name::DOT.is_dot() && Name::DOT_DOT.is_dot());
    assert!(!Name::new("A.B").expect("name").is_dot());
}

#[test]
fn a_deleted_entry_is_walked_past_and_its_slot_taken_again() {
    let mut fs = volume();
    let root = fs.root();
    let first = Name::new("FIRST.BIN").expect("name");
    let second = Name::new("SECOND.BIN").expect("name");
    fs.create(root, &first, moment()).expect("create");
    fs.create(root, &second, moment()).expect("create");
    assert_eq!(fs.remove(root, &first), Ok(0));
    let mut cursor = fs.entries(root);
    let entry = fs.next_entry(&mut cursor).expect("walk").expect("an entry");
    assert_eq!(entry.name, second);
    assert_eq!(fs.next_entry(&mut cursor).expect("walk"), None);
    let third = Name::new("THIRD.BIN").expect("name");
    let file = fs.create(root, &third, moment()).expect("create");
    assert_eq!(file.size(), 0);
    assert_eq!(
        fs.find(root, &third).expect("find").map(|e| e.name),
        Some(third)
    );
}

#[test]
fn the_volume_label_and_a_long_name_are_walked_past() {
    let mut fs = volume();
    let mut label = [0u8; ENTRY_LEN];
    put(&mut label, 0, b"AUDHSOS    ");
    put(&mut label, 11, &[ATTR_VOLUME_ID]);
    put_slot(&mut fs, 0, &label);
    let mut long = [0u8; ENTRY_LEN];
    put(&mut long, 0, &[0x41]);
    put(&mut long, 11, &[ATTR_LONG_NAME]);
    put_slot(&mut fs, 1, &long);
    let mut entry = [0u8; ENTRY_LEN];
    put(&mut entry, 0, b"REAL    BIN");
    put(&mut entry, 11, &[ATTR_ARCHIVE]);
    put(
        &mut entry,
        24,
        &crate::time::to_entry(moment())
            .expect("time")
            .0
            .to_le_bytes(),
    );
    put_slot(&mut fs, 2, &entry);
    let root = fs.root();
    let mut cursor = fs.entries(root);
    let found = fs.next_entry(&mut cursor).expect("walk").expect("an entry");
    assert_eq!(found.name, Name::new("REAL.BIN").expect("name"));
    assert_eq!(fs.next_entry(&mut cursor).expect("walk"), None);
}

#[test]
fn an_entry_whose_name_is_not_one_is_refused() {
    let mut fs = volume();
    let mut entry = [0u8; ENTRY_LEN];
    put(&mut entry, 0, b"lower   bin");
    put(&mut entry, 11, &[ATTR_ARCHIVE]);
    put_slot(&mut fs, 0, &entry);
    let root = fs.root();
    let mut cursor = fs.entries(root);
    assert_eq!(fs.next_entry(&mut cursor), Err(Error::EntryName));
}

#[test]
fn an_entry_whose_date_names_no_day_is_refused() {
    let mut fs = volume();
    let mut entry = [0u8; ENTRY_LEN];
    put(&mut entry, 0, b"BAD     BIN");
    put(&mut entry, 11, &[ATTR_ARCHIVE]);
    put(&mut entry, 24, &0u16.to_le_bytes());
    put_slot(&mut fs, 0, &entry);
    let root = fs.root();
    let mut cursor = fs.entries(root);
    assert_eq!(fs.next_entry(&mut cursor), Err(Error::Time));
}

#[test]
fn a_directory_grows_past_its_first_cluster_and_every_entry_reads_back() {
    let mut fs = volume();
    let root = fs.root();
    let names: Vec<Name> = (0..40u32)
        .map(|index| Name::new(&std::format!("F{index:03}.BIN")).expect("name"))
        .collect();
    for name in &names {
        fs.create(root, name, moment()).expect("create");
    }
    assert!(fs.chain_length(root.cluster()).expect("chain") >= 3);
    let mut cursor = fs.entries(root);
    let mut seen = Vec::new();
    while let Some(entry) = fs.next_entry(&mut cursor).expect("walk") {
        seen.push(entry.name);
    }
    assert_eq!(seen, names);
    for name in &names {
        assert!(fs.find(root, name).expect("find").is_some(), "{name}");
    }
}

#[test]
fn a_directory_carries_the_two_entries_it_keeps_for_itself() {
    let mut fs = volume();
    let root = fs.root();
    let efi = Name::new("EFI").expect("name");
    let boot = Name::new("BOOT").expect("name");
    let outer = fs.create_dir(root, &efi, moment()).expect("create dir");
    let inner = fs.create_dir(outer, &boot, moment()).expect("create dir");
    let sector = fs.geometry().cluster_sector(inner.cluster());
    let mut buffer = [0u8; SECTOR];
    fs.device().read(sector, &mut buffer).expect("read");
    assert_eq!(&buffer[..11], Name::DOT.as_bytes());
    assert_eq!(&buffer[ENTRY_LEN..ENTRY_LEN + 11], Name::DOT_DOT.as_bytes());
    assert_eq!(
        u32::from(u16::from_le_bytes([
            buffer[ENTRY_LEN + 26],
            buffer[ENTRY_LEN + 27]
        ])),
        outer.cluster(),
        "the parent of a directory below the root is that directory"
    );
    let mut top = [0u8; SECTOR];
    let top_sector = fs.geometry().cluster_sector(outer.cluster());
    fs.device().read(top_sector, &mut top).expect("read");
    assert_eq!(
        u16::from_le_bytes([top[ENTRY_LEN + 26], top[ENTRY_LEN + 27]]),
        0,
        "the parent of a directory in the root is written as zero"
    );
    // The dot entries are the directory's own and are not walked over.
    let mut cursor = fs.entries(inner);
    assert_eq!(fs.next_entry(&mut cursor).expect("walk"), None);
}

#[test]
fn a_name_that_is_taken_is_refused_and_one_that_is_missing_is_too() {
    let mut fs = volume();
    let root = fs.root();
    let name = Name::new("ONCE.BIN").expect("name");
    fs.create(root, &name, moment()).expect("create");
    assert_eq!(fs.create(root, &name, moment()).err(), Some(Error::Exists));
    assert_eq!(
        fs.create_dir(root, &name, moment()).err(),
        Some(Error::Exists)
    );
    let missing = Name::new("NOPE.BIN").expect("name");
    assert_eq!(fs.open(root, &missing).err(), Some(Error::NotFound));
    assert_eq!(fs.open_dir(root, &missing).err(), Some(Error::NotFound));
}

#[test]
fn a_file_is_not_a_directory_and_a_directory_is_not_a_file() {
    let mut fs = volume();
    let root = fs.root();
    let file = Name::new("A.BIN").expect("name");
    let folder = Name::new("SUB").expect("name");
    fs.create(root, &file, moment()).expect("create");
    fs.create_dir(root, &folder, moment()).expect("create dir");
    assert_eq!(fs.open_dir(root, &file).err(), Some(Error::Kind));
    assert_eq!(fs.open(root, &folder).err(), Some(Error::Kind));
    assert_eq!(
        fs.open_or_create_dir(root, &file, moment()).err(),
        Some(Error::Kind)
    );
    let again = fs
        .open_or_create_dir(root, &folder, moment())
        .expect("open");
    assert_eq!(again, fs.open_dir(root, &folder).expect("open dir"));
    let made = fs
        .open_or_create_dir(root, &Name::new("NEW").expect("name"), moment())
        .expect("create");
    assert!(made.cluster() > root.cluster());
    let entry = fs.find(root, &folder).expect("find").expect("an entry");
    assert!(entry.is_directory() && entry.attributes & ATTR_DIRECTORY != 0);
    assert_eq!(entry.size, 0);
}

#[test]
fn a_slot_a_deleted_entry_left_is_found_before_the_end_of_the_directory() {
    let mut fs = volume();
    let root = fs.root();
    let mut deleted = [0u8; ENTRY_LEN];
    put(&mut deleted, 0, &[DELETED]);
    put_slot(&mut fs, 0, &deleted);
    let name = Name::new("TAKEN.BIN").expect("name");
    fs.create(root, &name, moment()).expect("create");
    let entry = fs.find(root, &name).expect("find").expect("an entry");
    assert_eq!(entry.location.cluster(), root.cluster());
    assert_eq!(entry.location.slot(), 0);
}
