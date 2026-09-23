// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

//! Tests of `crate::open`.

use fs_fat::{Dir, Name};
use user_proto::file::ROOT;

use crate::open::{Clients, MAX_CLIENTS, MAX_OPEN, Opened, Which};
use crate::tests::support::{now, volume};

/// Something a client can hold open, which is the cheapest of the two.
fn directory(cluster: u32) -> Opened {
    let fs = volume();
    let dir = Dir::at(cluster);
    Opened::Dir {
        volume: Which::Written,
        dir,
        walk: fs.entries(dir),
        handed: 0,
    }
}

#[test]
fn a_server_nobody_opened_anything_at_holds_no_table() {
    let clients = Clients::new();
    assert!(clients.is_empty());
    assert_eq!(clients.len(), 0);
}

#[test]
fn a_handle_answers_under_the_badge_it_was_given_to_and_under_no_other() {
    let mut clients = Clients::new();
    let handle = clients.insert(7, directory(2)).unwrap();
    assert!(clients.get(7, handle).is_some());
    assert!(clients.get(8, handle).is_none(), "another client's badge");
    assert_eq!(clients.len(), 1);
}

#[test]
fn the_root_is_no_slot_of_any_table() {
    let mut clients = Clients::new();
    let handle = clients.insert(1, directory(2)).unwrap();
    assert_ne!(handle, ROOT, "the root occupies no slot");
    assert!(clients.get(1, ROOT).is_none());
    assert!(!clients.remove(1, ROOT));
}

#[test]
fn a_client_that_closes_its_last_file_keeps_no_table() {
    let mut clients = Clients::new();
    let handle = clients.insert(3, directory(2)).unwrap();
    assert_eq!(clients.len(), 1);
    assert!(clients.remove(3, handle));
    assert!(clients.is_empty(), "the table went with the last file");
    assert!(!clients.remove(3, handle), "and it cannot be closed twice");
}

#[test]
fn a_client_holds_as_many_as_the_table_has_slots_and_no_more() {
    let mut clients = Clients::new();
    for index in 0..MAX_OPEN {
        let cluster = u32::try_from(index).unwrap().wrapping_add(2);
        assert!(clients.insert(1, directory(cluster)).is_some(), "{index}");
    }
    assert!(clients.insert(1, directory(99)).is_none(), "one too many");
}

#[test]
fn the_server_keeps_as_many_tables_as_it_has_and_no_more() {
    let mut clients = Clients::new();
    for badge in 0..MAX_CLIENTS {
        let badge = u64::try_from(badge).unwrap().wrapping_add(1);
        assert!(clients.insert(badge, directory(2)).is_some(), "{badge}");
    }
    assert_eq!(clients.len(), MAX_CLIENTS);
    let one_more = u64::try_from(MAX_CLIENTS).unwrap().wrapping_add(1);
    assert!(clients.insert(one_more, directory(2)).is_none());
}

#[test]
fn a_client_that_is_gone_leaves_nothing_behind() {
    let mut clients = Clients::new();
    let handle = clients.insert(5, directory(2)).unwrap();
    clients.forget(5);
    assert!(clients.is_empty());
    assert!(clients.get(5, handle).is_none());
}

#[test]
fn a_handle_no_table_is_that_long_for_answers_nothing() {
    let mut clients = Clients::new();
    let _held = clients.insert(1, directory(2)).unwrap();
    let beyond = u32::try_from(MAX_OPEN).unwrap().wrapping_add(1);
    assert!(clients.get(1, beyond).is_none());
    assert!(!clients.remove(1, beyond));
    assert!(clients.get(1, u32::MAX).is_none());
}

#[test]
fn what_a_directory_handle_stands_for_is_a_directory_and_a_file_handle_is_not() {
    let mut clients = Clients::new();
    let handle = clients.insert(1, directory(9)).unwrap();
    let opened = clients.get(1, handle).unwrap();
    assert_eq!(opened.as_dir(), Some(Dir::at(9)));
}

#[test]
fn open_file_identity_includes_volume_parent_and_name() {
    let mut fs = volume();
    let root = fs.root();
    let first = Name::new("A.TXT").unwrap();
    let second = Name::new("B.TXT").unwrap();
    let file_a = fs.create(root, &first, now()).unwrap();
    let file_b = fs.create(root, &second, now()).unwrap();
    let mut clients = Clients::new();
    assert!(
        clients
            .insert_file(1, Which::Written, file_a, root, first)
            .is_some()
    );
    assert!(
        clients
            .insert_file(2, Which::Written, file_b, root, second)
            .is_some()
    );
    assert!(clients.has_open_file(Which::Written, root, first));
    assert!(!clients.has_open_file(Which::Written, root, Name::new("C.TXT").unwrap()));
    assert!(!clients.has_open_file(
        Which::Written,
        Dir::at(root.cluster().wrapping_add(1)),
        first
    ));
    assert!(!clients.has_open_file(Which::Booted, root, first));
}

#[test]
fn closing_a_root_walk_keeps_other_handles_and_releases_an_idle_table() {
    let fs = volume();
    let mut clients = Clients::new();
    let root = fs.root();
    let handle = clients.insert(1, directory(3)).unwrap();
    assert!(clients.root_walk(1, 0, || fs.entries(root)).is_some());
    assert!(clients.clear_root(1, 0));
    assert_eq!(clients.len(), 1);
    assert!(!clients.clear_root(1, 0));
    assert!(clients.remove(1, handle));
    assert!(clients.is_empty());
    assert!(!clients.clear_root(1, 0));
}
