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

/// The thirty-two bytes of an entry with a valid date, built by hand so
/// that `name` and `first_cluster` may be what no writer of this crate
/// would write.
fn raw(name: &[u8; 11], attributes: u8, first_cluster: u32, size: u32) -> [u8; ENTRY_LEN] {
    let (date, _) = crate::time::to_entry(moment()).expect("time");
    let mut entry = [0u8; ENTRY_LEN];
    put(&mut entry, 0, name);
    put(&mut entry, 11, &[attributes]);
    let cluster = first_cluster.to_le_bytes();
    put(&mut entry, 20, &cluster[2..]);
    put(&mut entry, 24, &date.to_le_bytes());
    put(&mut entry, 26, &cluster[..2]);
    put(&mut entry, 28, &size.to_le_bytes());
    entry
}

#[test]
fn an_entry_whose_name_or_date_this_crate_cannot_carry_is_walked_past() {
    let mut fs = volume();
    put_slot(&mut fs, 0, &raw(b"lower   bin", ATTR_ARCHIVE, 0, 0));
    put_slot(&mut fs, 1, &raw(b"\x05ANJI   BIN", ATTR_ARCHIVE, 0, 0));
    put_slot(&mut fs, 2, &raw(b"CODE\x80   BIN", ATTR_ARCHIVE, 0, 0));
    let mut undated = raw(b"BAD     BIN", ATTR_ARCHIVE, 0, 0);
    put(&mut undated, 24, &0u16.to_le_bytes());
    put_slot(&mut fs, 3, &undated);
    put_slot(&mut fs, 4, &raw(b"GOOD    BIN", ATTR_ARCHIVE, 0, 0));
    let root = fs.root();
    let good = Name::new("GOOD.BIN").expect("name");
    let mut cursor = fs.entries(root);
    let found = fs.next_entry(&mut cursor).expect("walk").expect("an entry");
    assert_eq!(found.name, good);
    assert_eq!(fs.next_entry(&mut cursor).expect("walk"), None);
    assert_eq!(
        fs.find(root, &good).expect("find").map(|e| e.name),
        Some(good)
    );
}

#[test]
fn a_name_a_walked_past_entry_carries_is_taken() {
    let mut fs = volume();
    let mut undated = raw(b"BAD     BIN", ATTR_ARCHIVE, 0, 0);
    put(&mut undated, 24, &0u16.to_le_bytes());
    put_slot(&mut fs, 0, &undated);
    put_slot(&mut fs, 1, &raw(b"lower   bin", ATTR_ARCHIVE, 0, 0));
    let root = fs.root();
    for text in ["BAD.BIN", "LOWER.BIN"] {
        let name = Name::new(text).expect("name");
        assert_eq!(fs.create(root, &name, moment()), Err(Error::Exists));
        assert_eq!(fs.create_dir(root, &name, moment()), Err(Error::Exists));
    }
}

#[test]
fn an_entry_whose_first_cluster_is_outside_the_data_region_is_refused() {
    let mut fs = volume();
    let last = fs.geometry().last_cluster();
    put_slot(&mut fs, 0, &raw(b"ONE     BIN", ATTR_ARCHIVE, 1, 512));
    put_slot(&mut fs, 1, &raw(b"ZERO       ", ATTR_DIRECTORY, 0, 0));
    put_slot(
        &mut fs,
        2,
        &raw(b"PAST    BIN", ATTR_ARCHIVE, last + 1, 512),
    );
    put_slot(&mut fs, 3, &raw(b"EMPTY   BIN", ATTR_ARCHIVE, 0, 0));
    let root = fs.root();
    let mut cursor = fs.entries(root);
    // Each refusal leaves the walk behind the entry it refused (#215).
    assert_eq!(fs.next_entry(&mut cursor), Err(Error::Cluster(1)));
    assert_eq!(fs.next_entry(&mut cursor), Err(Error::Cluster(0)));
    assert_eq!(fs.next_entry(&mut cursor), Err(Error::Cluster(last + 1)));
    let empty = fs.next_entry(&mut cursor).expect("walk").expect("an entry");
    assert_eq!(empty.name, Name::new("EMPTY.BIN").expect("name"));
    assert_eq!(fs.next_entry(&mut cursor).expect("walk"), None);
    let one = Name::new("ONE.BIN").expect("name");
    assert_eq!(fs.open(root, &one), Err(Error::Cluster(1)));
    let zero = Name::new("ZERO").expect("name");
    assert_eq!(fs.open_dir(root, &zero), Err(Error::Cluster(0)));
    // `find` refuses only the entry it names; the others stay reachable.
    let empty = Name::new("EMPTY.BIN").expect("name");
    assert!(fs.open(root, &empty).is_ok());
    let new = Name::new("NEW.BIN").expect("name");
    assert!(fs.create(root, &new, moment()).is_ok());
    assert_eq!(fs.create(root, &one, moment()), Err(Error::Exists));
}

/// A device that counts the sectors read from it.
struct Counting {
    disk: crate::doubles::RamDisk,
    reads: core::cell::Cell<u32>,
}

impl BlockDevice for Counting {
    fn sectors(&self) -> u32 {
        self.disk.sectors()
    }

    fn read(&self, sector: u32, into: &mut [u8; SECTOR]) -> Result<(), Error> {
        self.reads.set(self.reads.get().saturating_add(1));
        self.disk.read(sector, into)
    }

    fn write(&mut self, sector: u32, from: &[u8; SECTOR]) -> Result<(), Error> {
        self.disk.write(sector, from)
    }
}

#[test]
fn a_walk_reads_each_directory_sector_once() {
    let mut fs = volume();
    let root = fs.root();
    for index in 0..32u32 {
        let name = Name::new(&std::format!("F{index:03}.BIN")).expect("name");
        fs.create(root, &name, moment()).expect("create");
    }
    let disk = Counting {
        disk: fs.into_device(),
        reads: core::cell::Cell::new(0),
    };
    let fs = FileSystem::mount(disk).expect("mount");
    assert_eq!(fs.chain_length(root.cluster()), Ok(2));
    fs.device().reads.set(0);
    let mut cursor = fs.entries(root);
    let mut seen = 0u32;
    while fs.next_entry(&mut cursor).expect("walk").is_some() {
        seen += 1;
    }
    assert_eq!(seen, 32);
    // Two directory sectors and one table read per cluster (#224).
    assert_eq!(fs.device().reads.get(), 4);
}

#[test]
fn a_walk_sees_what_was_written_into_the_sector_it_holds() {
    let mut fs = volume();
    let root = fs.root();
    let names: Vec<Name> = ["A.BIN", "B.BIN", "C.BIN"]
        .iter()
        .map(|text| Name::new(text).expect("name"))
        .collect();
    fs.create(root, &names[0], moment()).expect("create");
    fs.create(root, &names[1], moment()).expect("create");
    let mut cursor = fs.entries(root);
    let first = fs.next_entry(&mut cursor).expect("walk").expect("an entry");
    assert_eq!(first.name, names[0]);
    fs.create(root, &names[2], moment()).expect("create");
    let mut seen = std::vec![first.name];
    while let Some(entry) = fs.next_entry(&mut cursor).expect("walk") {
        seen.push(entry.name);
    }
    assert_eq!(seen, names);
    // A write through `device_mut` drops the cached sector as well.
    put_slot(&mut fs, 3, &raw(b"D       BIN", ATTR_ARCHIVE, 0, 0));
    let d = Name::new("D.BIN").expect("name");
    assert_eq!(fs.find(root, &d).expect("find").map(|e| e.name), Some(d));
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
