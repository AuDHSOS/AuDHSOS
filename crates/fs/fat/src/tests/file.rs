// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

//! Reading and writing the bytes of a file, and the moment its entry
//! carries.

use audhsos_time::{CivilTime, UnixTime};

use crate::error::Error;
use crate::name::Name;
use crate::tests::support::{moment, volume};
use crate::time;

/// A file of `bytes` recognisable bytes.
fn pattern(bytes: usize) -> Vec<u8> {
    (0..bytes)
        .map(|index| u8::try_from(index % 251).expect("byte"))
        .collect()
}

#[test]
fn what_is_written_is_what_is_read_back() {
    let mut fs = volume();
    let root = fs.root();
    let name = Name::new("DATA.BIN").expect("name");
    let data = pattern(5000);
    let mut file = fs.create(root, &name, moment()).expect("create");
    assert_eq!(fs.write(&mut file, 0, &data), Ok(data.len()));
    assert_eq!(file.size(), 5000);
    let mut back = vec![0u8; data.len()];
    let mut open = fs.open(root, &name).expect("open");
    assert_eq!(fs.read(&mut open, 0, &mut back), Ok(data.len()));
    assert_eq!(back, data);
}

#[test]
fn a_read_at_an_offset_and_across_a_cluster_boundary_reads_the_right_bytes() {
    let mut fs = volume();
    let root = fs.root();
    let name = Name::new("SPAN.BIN").expect("name");
    let data = pattern(2048);
    let mut file = fs.create(root, &name, moment()).expect("create");
    fs.write(&mut file, 0, &data).expect("write");
    let mut open = fs.open(root, &name).expect("open");
    let mut middle = [0u8; 8];
    assert_eq!(fs.read(&mut open, 500, &mut middle), Ok(8));
    assert_eq!(&middle, &data[500..508]);
    // Across the boundary at 512, which is where one cluster ends.
    let mut across = [0u8; 20];
    assert_eq!(fs.read(&mut open, 505, &mut across), Ok(20));
    assert_eq!(&across, &data[505..525]);
    // Backwards, which sends the cursor to the first cluster again.
    let mut back = [0u8; 4];
    assert_eq!(fs.read(&mut open, 4, &mut back), Ok(4));
    assert_eq!(&back, &data[4..8]);
}

#[test]
fn a_read_at_the_end_reads_nothing_and_one_past_it_is_refused() {
    let mut fs = volume();
    let root = fs.root();
    let name = Name::new("END.BIN").expect("name");
    let mut file = fs.create(root, &name, moment()).expect("create");
    fs.write(&mut file, 0, &pattern(100)).expect("write");
    let mut open = fs.open(root, &name).expect("open");
    let mut buffer = [0u8; 16];
    assert_eq!(fs.read(&mut open, 100, &mut buffer), Ok(0));
    assert_eq!(fs.read(&mut open, 90, &mut buffer), Ok(10));
    assert_eq!(
        fs.read(&mut open, 101, &mut buffer),
        Err(Error::Offset(101))
    );
    assert_eq!(fs.write(&mut open, 101, &[1]), Err(Error::Offset(101)));
}

#[test]
fn a_read_from_a_file_with_no_cluster_reads_nothing() {
    let mut fs = volume();
    let root = fs.root();
    let name = Name::new("NONE.BIN").expect("name");
    let mut file = fs.create(root, &name, moment()).expect("create");
    let mut buffer = [0u8; 8];
    assert_eq!(fs.read(&mut file, 0, &mut buffer), Ok(0));
    assert_eq!(file.first_cluster(), 0);
    assert_eq!(fs.write(&mut file, 0, &[]), Ok(0));
    assert_eq!(
        file.first_cluster(),
        0,
        "a write of nothing takes no cluster"
    );
}

#[test]
fn a_write_that_extends_the_file_keeps_what_was_there() {
    let mut fs = volume();
    let root = fs.root();
    let name = Name::new("GROW.BIN").expect("name");
    let mut file = fs.create(root, &name, moment()).expect("create");
    fs.write(&mut file, 0, &[1u8; 100]).expect("write");
    fs.write(&mut file, 100, &[2u8; 900]).expect("extend");
    assert_eq!(file.size(), 1000);
    assert_eq!(fs.chain_length(file.first_cluster()), Ok(2));
    let mut back = vec![0u8; 1000];
    let mut open = fs.open(root, &name).expect("open");
    assert_eq!(fs.read(&mut open, 0, &mut back), Ok(1000));
    assert_eq!(&back[..100], &[1u8; 100]);
    assert_eq!(&back[100..], &[2u8; 900][..]);
}

#[test]
fn a_write_that_overwrites_changes_only_what_it_covers() {
    let mut fs = volume();
    let root = fs.root();
    let name = Name::new("OVER.BIN").expect("name");
    let data = pattern(1500);
    let mut file = fs.create(root, &name, moment()).expect("create");
    fs.write(&mut file, 0, &data).expect("write");
    fs.write(&mut file, 600, &[0xFF; 10]).expect("overwrite");
    assert_eq!(file.size(), 1500, "an overwrite does not change the size");
    let mut back = vec![0u8; 1500];
    let mut open = fs.open(root, &name).expect("open");
    fs.read(&mut open, 0, &mut back).expect("read");
    assert_eq!(&back[..600], &data[..600]);
    assert_eq!(&back[600..610], &[0xFF; 10]);
    assert_eq!(&back[610..], &data[610..]);
}

#[test]
fn a_write_that_fills_the_last_cluster_exactly_takes_no_further_one() {
    let mut fs = volume();
    let root = fs.root();
    let name = Name::new("EXACT.BIN").expect("name");
    let mut file = fs.create(root, &name, moment()).expect("create");
    fs.write(&mut file, 0, &[3u8; 1024]).expect("write");
    assert_eq!(fs.chain_length(file.first_cluster()), Ok(2));
    let free = fs.free_clusters();
    fs.write(&mut file, 1024, &[4u8; 512]).expect("one more");
    assert_eq!(fs.chain_length(file.first_cluster()), Ok(3));
    assert_eq!(fs.free_clusters(), free - 1);
    assert_eq!(file.size(), 1536);
}

#[test]
fn a_file_written_in_many_calls_is_the_file_written_in_one() {
    let mut fs = volume();
    let root = fs.root();
    let whole = Name::new("WHOLE.BIN").expect("name");
    let parts = Name::new("PARTS.BIN").expect("name");
    let data = pattern(4000);
    let mut one = fs.create(root, &whole, moment()).expect("create");
    fs.write(&mut one, 0, &data).expect("write");
    let mut many = fs.create(root, &parts, moment()).expect("create");
    let mut at = 0u32;
    for chunk in data.chunks(37) {
        fs.write(&mut many, at, chunk).expect("write");
        at += u32::try_from(chunk.len()).expect("length");
    }
    assert_eq!(one.size(), many.size());
    let mut left = vec![0u8; data.len()];
    let mut right = vec![0u8; data.len()];
    let mut open_one = fs.open(root, &whole).expect("open");
    let mut open_many = fs.open(root, &parts).expect("open");
    fs.read(&mut open_one, 0, &mut left).expect("read");
    fs.read(&mut open_many, 0, &mut right).expect("read");
    assert_eq!(left, right);
    assert_eq!(left, data);
}

#[test]
fn the_moment_an_entry_carries_reads_back_as_the_moment_it_was_given() {
    let mut fs = volume();
    let root = fs.root();
    let name = Name::new("WHEN.BIN").expect("name");
    let at = moment();
    fs.create(root, &name, at).expect("create");
    let entry = fs.find(root, &name).expect("find").expect("an entry");
    assert_eq!(entry.modified, at);
}

#[test]
fn every_even_second_of_the_range_round_trips_and_nothing_else_does() {
    for civil in [
        CivilTime::new(1980, 1, 1, 0, 0, 0),
        CivilTime::new(2026, 9, 7, 13, 45, 30),
        CivilTime::new(2107, 12, 31, 23, 59, 58),
    ] {
        let at = UnixTime::from_civil(civil.expect("civil")).expect("unix");
        let (date, clock) = time::to_entry(at).expect("to entry");
        assert_eq!(time::from_entry(date, clock), Ok(at));
    }
    let odd =
        UnixTime::from_civil(CivilTime::new(2026, 1, 1, 0, 0, 1).expect("civil")).expect("unix");
    assert_eq!(time::to_entry(odd), Err(Error::Time));
    let early =
        UnixTime::from_civil(CivilTime::new(1979, 12, 31, 0, 0, 0).expect("civil")).expect("unix");
    assert_eq!(time::to_entry(early), Err(Error::Time));
    let late =
        UnixTime::from_civil(CivilTime::new(2108, 1, 1, 0, 0, 0).expect("civil")).expect("unix");
    assert_eq!(time::to_entry(late), Err(Error::Time));
    assert_eq!(
        time::to_entry(UnixTime::from_seconds(i64::MAX)),
        Err(Error::Time)
    );
    // A month of zero, a day of zero, and an hour of twenty-four name no
    // moment.
    assert_eq!(time::from_entry(0, 0), Err(Error::Time));
    assert_eq!(time::from_entry((46 << 9) | (1 << 5), 0), Err(Error::Time));
    assert_eq!(
        time::from_entry((46 << 9) | (1 << 5) | 1, 24 << 11),
        Err(Error::Time)
    );
    assert_eq!(
        time::from_entry((46 << 9) | (13 << 5) | 1, 0),
        Err(Error::Time)
    );
}

#[test]
fn a_write_of_nothing_inside_a_file_takes_no_cluster_and_changes_no_size() {
    let mut fs = volume();
    let root = fs.root();
    let name = Name::new("NOTHING.BIN").expect("name");
    let mut file = fs.create(root, &name, moment()).expect("create");
    fs.write(&mut file, 0, &[1u8; 10]).expect("write");
    let free = fs.free_clusters();
    assert_eq!(fs.write(&mut file, 5, &[]), Ok(0));
    assert_eq!(fs.free_clusters(), free);
    assert_eq!(file.size(), 10);
}

#[test]
fn a_moment_in_an_entry_is_not_moved_by_a_write() {
    let mut fs = volume();
    let root = fs.root();
    let name = Name::new("STILL.BIN").expect("name");
    let at = moment();
    let mut file = fs.create(root, &name, at).expect("create");
    fs.write(&mut file, 0, &pattern(900)).expect("write");
    let entry = fs.find(root, &name).expect("find").expect("an entry");
    assert_eq!(entry.modified, at);
    assert_eq!(entry.size, 900);
}

#[test]
fn a_file_made_with_a_moment_an_entry_cannot_carry_is_refused() {
    let mut fs = volume();
    let root = fs.root();
    let name = Name::new("EARLY.BIN").expect("name");
    let before =
        UnixTime::from_civil(CivilTime::new(1970, 1, 1, 0, 0, 0).expect("civil")).expect("unix");
    assert_eq!(fs.create(root, &name, before), Err(Error::Time));
    assert_eq!(fs.create_dir(root, &name, before), Err(Error::Time));
}
