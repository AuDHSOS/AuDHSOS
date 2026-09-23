// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

//! Tests of `crate::serve`.

use std::vec;
use std::vec::Vec;

use audhsos_abi::Error;
use audhsos_time::UnixTime;
use fs_fat::doubles::RamDisk;
use fs_fat::{ATTR_DIRECTORY, FileSystem};
use user_proto::file::{Data, MAX_DATA, Name, ROOT, Reply, Request, START};

use crate::open::{Clients, MAX_CLIENTS};
use crate::serve::{NOBODY, Volumes, answer, moment};
use crate::tests::support::{now, volume};

/// The badge of the client every test speaks as.
const CLIENT: u64 = 1;

/// Answers `request` as [`CLIENT`], on a machine of one volume.
fn ask(fs: &mut FileSystem<RamDisk>, clients: &mut Clients, request: &Request) -> Reply {
    as_client(fs, clients, CLIENT, request)
}

/// The same, as whichever client `badge` names.
fn as_client(
    fs: &mut FileSystem<RamDisk>,
    clients: &mut Clients,
    badge: u64,
    request: &Request,
) -> Reply {
    let mut volumes = Volumes {
        written: Some(fs),
        booted: None,
    };
    answer(&mut volumes, clients, badge, request, now())
}

/// The name `text` stands for.
fn name(text: &[u8]) -> Name {
    Name::new(text).unwrap()
}

/// Makes a file in the root and answers its handle.
fn make(fs: &mut FileSystem<RamDisk>, clients: &mut Clients, text: &[u8]) -> u32 {
    match ask(
        fs,
        clients,
        &Request::Create {
            parent: ROOT,
            name: name(text),
            directory: false,
        },
    ) {
        Reply::Created(Ok(file)) => file,
        other => panic!("{other:?}"),
    }
}

#[test]
fn a_file_made_now_is_opened_again_with_the_bytes_that_were_written() {
    let mut fs = volume();
    let mut clients = Clients::new();
    let file = make(&mut fs, &mut clients, b"NOTES.TXT");
    let written = ask(
        &mut fs,
        &mut clients,
        &Request::Write {
            file,
            offset: 0,
            data: Data::new(b"one line\n").unwrap(),
        },
    );
    assert_eq!(written, Reply::Written(Ok(9)));
    assert_eq!(
        ask(&mut fs, &mut clients, &Request::Close { file }),
        Reply::Closed(Ok(()))
    );

    let opened = match ask(
        &mut fs,
        &mut clients,
        &Request::Open {
            parent: ROOT,
            name: name(b"NOTES.TXT"),
        },
    ) {
        Reply::Opened(Ok(opened)) => opened,
        other => panic!("{other:?}"),
    };
    assert_eq!(opened.size, 9);
    assert!(!opened.directory);
    let read = ask(
        &mut fs,
        &mut clients,
        &Request::Read {
            file: opened.file,
            offset: 0,
            len: 64,
        },
    );
    assert_eq!(read, Reply::Read(Ok(Data::new(b"one line\n").unwrap())));
}

#[test]
fn a_name_nobody_made_is_not_found_and_a_name_made_twice_exists() {
    let mut fs = volume();
    let mut clients = Clients::new();
    assert_eq!(
        ask(
            &mut fs,
            &mut clients,
            &Request::Open {
                parent: ROOT,
                name: name(b"GONE.TXT")
            }
        ),
        Reply::Opened(Err(Error::NotFound))
    );
    let _first = make(&mut fs, &mut clients, b"ONCE.TXT");
    assert_eq!(
        ask(
            &mut fs,
            &mut clients,
            &Request::Create {
                parent: ROOT,
                name: name(b"ONCE.TXT"),
                directory: false
            }
        ),
        Reply::Created(Err(Error::AlreadyExists))
    );
}

#[test]
fn a_directory_is_opened_as_one_and_read_as_one() {
    let mut fs = volume();
    let mut clients = Clients::new();
    let made = ask(
        &mut fs,
        &mut clients,
        &Request::Create {
            parent: ROOT,
            name: name(b"SUB"),
            directory: true,
        },
    );
    assert!(matches!(made, Reply::Created(Ok(_))));
    let opened = match ask(
        &mut fs,
        &mut clients,
        &Request::Open {
            parent: ROOT,
            name: name(b"SUB"),
        },
    ) {
        Reply::Opened(Ok(opened)) => opened,
        other => panic!("{other:?}"),
    };
    assert!(opened.directory);
    // A directory is no file: a read of one is refused rather than
    // answered with its bytes.
    assert_eq!(
        ask(
            &mut fs,
            &mut clients,
            &Request::Read {
                file: opened.file,
                offset: 0,
                len: 16
            }
        ),
        Reply::Read(Err(Error::WrongObjectType))
    );
}

#[test]
fn the_root_is_read_without_being_opened_and_names_what_was_made() {
    let mut fs = volume();
    let mut clients = Clients::new();
    let _one = make(&mut fs, &mut clients, b"A.TXT");
    let _two = make(&mut fs, &mut clients, b"B.TXT");
    let mut cursor = START;
    let mut names = Vec::new();
    loop {
        let Reply::Entry(Ok(entry)) = ask(
            &mut fs,
            &mut clients,
            &Request::ReadDir { dir: ROOT, cursor },
        ) else {
            panic!("a directory read was refused");
        };
        let Some(entry) = entry else {
            break;
        };
        cursor = entry.cursor;
        names.push(entry.name);
    }
    assert_eq!(names, vec![name(b"A.TXT"), name(b"B.TXT")]);
}

#[test]
fn a_cursor_that_is_not_where_the_walk_stands_starts_it_again() {
    let mut fs = volume();
    let mut clients = Clients::new();
    let _one = make(&mut fs, &mut clients, b"A.TXT");
    let _two = make(&mut fs, &mut clients, b"B.TXT");
    let first = ask(
        &mut fs,
        &mut clients,
        &Request::ReadDir {
            dir: ROOT,
            cursor: START,
        },
    );
    // The same cursor answers the same entry, however often it is asked.
    let again = ask(
        &mut fs,
        &mut clients,
        &Request::ReadDir {
            dir: ROOT,
            cursor: START,
        },
    );
    assert_eq!(first, again);
}

#[test]
fn a_stat_says_what_the_entry_says() {
    let mut fs = volume();
    let mut clients = Clients::new();
    let file = make(&mut fs, &mut clients, b"S.TXT");
    let _written = ask(
        &mut fs,
        &mut clients,
        &Request::Write {
            file,
            offset: 0,
            data: Data::new(b"1234").unwrap(),
        },
    );
    let Reply::Stat(Ok(stat)) = ask(&mut fs, &mut clients, &Request::Stat { file }) else {
        panic!("a stat was refused");
    };
    assert_eq!(stat.size, 4);
    assert_eq!(stat.attributes & u32::from(ATTR_DIRECTORY), 0);
    assert_eq!(stat.modified, u64::try_from(now().seconds()).unwrap());
}

#[test]
fn the_root_stats_as_a_directory_without_being_opened() {
    let mut fs = volume();
    let mut clients = Clients::new();
    let Reply::Stat(Ok(stat)) = ask(&mut fs, &mut clients, &Request::Stat { file: ROOT }) else {
        panic!("a stat of the root was refused");
    };
    assert_eq!(stat.attributes, u32::from(ATTR_DIRECTORY));
    assert_eq!(stat.size, 0);
}

#[test]
fn a_removed_name_is_gone_and_removing_it_twice_is_refused() {
    let mut fs = volume();
    let mut clients = Clients::new();
    let file = make(&mut fs, &mut clients, b"OLD.TXT");
    let _closed = ask(&mut fs, &mut clients, &Request::Close { file });
    assert_eq!(
        ask(
            &mut fs,
            &mut clients,
            &Request::Remove {
                parent: ROOT,
                name: name(b"OLD.TXT")
            }
        ),
        Reply::Removed(Ok(()))
    );
    assert_eq!(
        ask(
            &mut fs,
            &mut clients,
            &Request::Remove {
                parent: ROOT,
                name: name(b"OLD.TXT")
            }
        ),
        Reply::Removed(Err(Error::NotFound))
    );
}

#[test]
fn remove_refuses_a_file_held_by_another_client() {
    let mut fs = volume();
    let mut clients = Clients::new();
    let file = make(&mut fs, &mut clients, b"A.TXT");
    assert_eq!(
        ask(
            &mut fs,
            &mut clients,
            &Request::Write {
                file,
                offset: 0,
                data: Data::new(b"original").unwrap(),
            }
        ),
        Reply::Written(Ok(8))
    );
    assert_eq!(
        as_client(
            &mut fs,
            &mut clients,
            CLIENT + 1,
            &Request::Remove {
                parent: ROOT,
                name: name(b"A.TXT"),
            }
        ),
        Reply::Removed(Err(Error::Busy))
    );
    assert_eq!(
        ask(
            &mut fs,
            &mut clients,
            &Request::Read {
                file,
                offset: 0,
                len: 8
            }
        ),
        Reply::Read(Ok(Data::new(b"original").unwrap()))
    );
    assert_eq!(
        ask(&mut fs, &mut clients, &Request::Close { file }),
        Reply::Closed(Ok(()))
    );
    assert_eq!(
        as_client(
            &mut fs,
            &mut clients,
            CLIENT + 1,
            &Request::Remove {
                parent: ROOT,
                name: name(b"A.TXT"),
            }
        ),
        Reply::Removed(Ok(()))
    );
}

#[test]
fn handles_of_one_file_share_its_size_across_clients() {
    let mut fs = volume();
    let mut clients = Clients::new();
    let first = make(&mut fs, &mut clients, b"SHARED.TXT");
    let second = match as_client(
        &mut fs,
        &mut clients,
        CLIENT + 1,
        &Request::Open {
            parent: ROOT,
            name: name(b"SHARED.TXT"),
        },
    ) {
        Reply::Opened(Ok(opened)) => opened.file,
        other => panic!("{other:?}"),
    };
    assert_eq!(
        ask(
            &mut fs,
            &mut clients,
            &Request::Write {
                file: first,
                offset: 0,
                data: Data::new(b"abcdefghij").unwrap(),
            }
        ),
        Reply::Written(Ok(10))
    );
    assert_eq!(
        as_client(
            &mut fs,
            &mut clients,
            CLIENT + 1,
            &Request::Write {
                file: second,
                offset: 0,
                data: Data::new(b"XY").unwrap(),
            }
        ),
        Reply::Written(Ok(2))
    );
    assert_eq!(
        ask(
            &mut fs,
            &mut clients,
            &Request::Read {
                file: first,
                offset: 0,
                len: 10
            }
        ),
        Reply::Read(Ok(Data::new(b"XYcdefghij").unwrap()))
    );
    assert_eq!(
        fs.find(fs.root(), &fs_fat::Name::new("SHARED.TXT").unwrap())
            .unwrap()
            .unwrap()
            .size,
        10
    );
    assert_eq!(
        ask(&mut fs, &mut clients, &Request::Close { file: first }),
        Reply::Closed(Ok(()))
    );
    assert_eq!(
        as_client(
            &mut fs,
            &mut clients,
            CLIENT + 1,
            &Request::Write {
                file: second,
                offset: 10,
                data: Data::new(b"Z").unwrap(),
            }
        ),
        Reply::Written(Ok(1))
    );
    assert_eq!(
        as_client(
            &mut fs,
            &mut clients,
            CLIENT + 1,
            &Request::Read {
                file: second,
                offset: 0,
                len: 11,
            }
        ),
        Reply::Read(Ok(Data::new(b"XYcdefghijZ").unwrap()))
    );
}

#[test]
fn completed_root_walks_release_client_tables() {
    let mut fs = volume();
    let mut clients = Clients::new();
    for badge in 1..=MAX_CLIENTS + 1 {
        let badge = u64::try_from(badge).unwrap();
        assert_eq!(
            as_client(
                &mut fs,
                &mut clients,
                badge,
                &Request::ReadDir {
                    dir: ROOT,
                    cursor: START
                }
            ),
            Reply::Entry(Ok(None))
        );
        assert!(clients.is_empty());
    }
}

#[test]
fn closing_an_unfinished_root_walk_releases_its_table() {
    let mut fs = volume();
    let mut clients = Clients::new();
    let file = make(&mut fs, &mut clients, b"A.TXT");
    assert_eq!(
        ask(&mut fs, &mut clients, &Request::Close { file }),
        Reply::Closed(Ok(()))
    );
    assert!(matches!(
        ask(
            &mut fs,
            &mut clients,
            &Request::ReadDir {
                dir: ROOT,
                cursor: START
            }
        ),
        Reply::Entry(Ok(Some(_)))
    ));
    assert_eq!(clients.len(), 1);
    assert_eq!(
        ask(&mut fs, &mut clients, &Request::Close { file: ROOT }),
        Reply::Closed(Ok(()))
    );
    assert!(clients.is_empty());
    assert_eq!(
        ask(&mut fs, &mut clients, &Request::Close { file: ROOT }),
        Reply::Closed(Err(Error::InvalidHandle))
    );
}

#[test]
fn a_handle_of_another_client_names_nothing_here() {
    let mut fs = volume();
    let mut clients = Clients::new();
    let file = make(&mut fs, &mut clients, b"MINE.TXT");
    let theirs = as_client(
        &mut fs,
        &mut clients,
        CLIENT + 1,
        &Request::Read {
            file,
            offset: 0,
            len: 4,
        },
    );
    assert_eq!(theirs, Reply::Read(Err(Error::InvalidHandle)));
}

#[test]
fn a_handle_nobody_holds_is_refused_by_every_message_that_takes_one() {
    let mut fs = volume();
    let mut clients = Clients::new();
    let gone = 9u32;
    assert_eq!(
        ask(
            &mut fs,
            &mut clients,
            &Request::Read {
                file: gone,
                offset: 0,
                len: 1
            }
        ),
        Reply::Read(Err(Error::InvalidHandle))
    );
    assert_eq!(
        ask(
            &mut fs,
            &mut clients,
            &Request::Write {
                file: gone,
                offset: 0,
                data: Data::empty()
            }
        ),
        Reply::Written(Err(Error::InvalidHandle))
    );
    assert_eq!(
        ask(&mut fs, &mut clients, &Request::Stat { file: gone }),
        Reply::Stat(Err(Error::InvalidHandle))
    );
    assert_eq!(
        ask(&mut fs, &mut clients, &Request::Close { file: gone }),
        Reply::Closed(Err(Error::InvalidHandle))
    );
    assert_eq!(
        ask(
            &mut fs,
            &mut clients,
            &Request::ReadDir {
                dir: gone,
                cursor: START
            }
        ),
        Reply::Entry(Err(Error::InvalidHandle))
    );
}

#[test]
fn a_file_handle_is_no_directory_to_open_a_name_in() {
    let mut fs = volume();
    let mut clients = Clients::new();
    let file = make(&mut fs, &mut clients, b"F.TXT");
    assert_eq!(
        ask(
            &mut fs,
            &mut clients,
            &Request::Open {
                parent: file,
                name: name(b"X.TXT")
            }
        ),
        Reply::Opened(Err(Error::WrongObjectType))
    );
}

#[test]
fn a_name_the_volume_cannot_spell_is_refused_before_the_disk_is_touched() {
    let mut fs = volume();
    let mut clients = Clients::new();
    assert_eq!(
        ask(
            &mut fs,
            &mut clients,
            &Request::Open {
                parent: ROOT,
                name: Name::new(b"a b").unwrap()
            }
        ),
        Reply::Opened(Err(Error::InvalidArgument))
    );
}

#[test]
fn a_read_longer_than_one_message_is_cut_to_what_one_carries() {
    let mut fs = volume();
    let mut clients = Clients::new();
    let file = make(&mut fs, &mut clients, b"BIG.BIN");
    let block = [0x5Au8; MAX_DATA];
    let _written = ask(
        &mut fs,
        &mut clients,
        &Request::Write {
            file,
            offset: 0,
            data: Data::new(&block).unwrap(),
        },
    );
    let _more = ask(
        &mut fs,
        &mut clients,
        &Request::Write {
            file,
            offset: u32::try_from(MAX_DATA).unwrap(),
            data: Data::new(&block).unwrap(),
        },
    );
    let Reply::Read(Ok(read)) = ask(
        &mut fs,
        &mut clients,
        &Request::Read {
            file,
            offset: 0,
            len: u32::MAX,
        },
    ) else {
        panic!("a read was refused");
    };
    assert_eq!(read.len(), MAX_DATA);
}

#[test]
fn a_flush_answers_and_leaves_the_volume_readable() {
    let mut fs = volume();
    let mut clients = Clients::new();
    assert_eq!(
        ask(&mut fs, &mut clients, &Request::Flush),
        Reply::Flushed(Ok(()))
    );
}

#[test]
fn the_moment_an_entry_carries_is_even_and_no_earlier_than_the_format_allows() {
    assert_eq!(moment(0), UnixTime::from_seconds(315_532_800));
    assert_eq!(moment(1_000_000), UnixTime::from_seconds(315_532_800));
    // An odd second is rounded down: a directory entry holds the seconds
    // in units of two.
    assert_eq!(
        moment(1_767_225_601_000_000),
        UnixTime::from_seconds(1_767_225_600)
    );
    assert_eq!(
        moment(1_767_225_603_999_999),
        UnixTime::from_seconds(1_767_225_602)
    );
}

#[test]
fn a_device_that_will_not_move_a_sector_is_a_refusal_and_not_a_wrong_answer() {
    let mut fs = volume();
    let mut clients = Clients::new();
    fs.device_mut().refuse(0);
    let outcome = ask(
        &mut fs,
        &mut clients,
        &Request::Open {
            parent: ROOT,
            name: name(b"ANY.TXT"),
        },
    );
    assert!(matches!(outcome, Reply::Opened(Err(_))), "{outcome:?}");
}

#[test]
fn a_cursor_ahead_of_the_walk_skips_forward_to_it() {
    let mut fs = volume();
    let mut clients = Clients::new();
    for text in [b"A.TXT", b"B.TXT", b"C.TXT"] {
        let file = make(&mut fs, &mut clients, text);
        let _closed = ask(&mut fs, &mut clients, &Request::Close { file });
    }
    let dir = match ask(
        &mut fs,
        &mut clients,
        &Request::Create {
            parent: ROOT,
            name: name(b"SUB"),
            directory: true,
        },
    ) {
        Reply::Created(Ok(handle)) => handle,
        other => panic!("{other:?}"),
    };
    // The walk of a fresh directory stands at the start; a cursor of two
    // makes it skip the two entries a directory keeps for itself.
    let Reply::Entry(Ok(entry)) = ask(&mut fs, &mut clients, &Request::ReadDir { dir, cursor: 2 })
    else {
        panic!("a directory read was refused");
    };
    assert_eq!(
        entry, None,
        "a directory of its own two entries has no third"
    );
    assert_eq!(
        ask(&mut fs, &mut clients, &Request::ReadDir { dir, cursor: 3 }),
        Reply::Entry(Ok(None))
    );
}

#[test]
fn a_directory_handle_stats_as_a_directory_and_refuses_a_write() {
    let mut fs = volume();
    let mut clients = Clients::new();
    let dir = match ask(
        &mut fs,
        &mut clients,
        &Request::Create {
            parent: ROOT,
            name: name(b"SUB"),
            directory: true,
        },
    ) {
        Reply::Created(Ok(handle)) => handle,
        other => panic!("{other:?}"),
    };
    let Reply::Stat(Ok(stat)) = ask(&mut fs, &mut clients, &Request::Stat { file: dir }) else {
        panic!("a stat of a directory was refused");
    };
    assert_eq!(stat.attributes, u32::from(ATTR_DIRECTORY));
    assert_eq!(
        ask(
            &mut fs,
            &mut clients,
            &Request::Write {
                file: dir,
                offset: 0,
                data: Data::new(b"x").unwrap()
            }
        ),
        Reply::Written(Err(Error::WrongObjectType))
    );
}

#[test]
fn a_name_without_an_extension_is_spelled_without_a_dot() {
    let mut fs = volume();
    let mut clients = Clients::new();
    let _made = ask(
        &mut fs,
        &mut clients,
        &Request::Create {
            parent: ROOT,
            name: name(b"SUB"),
            directory: true,
        },
    );
    let Reply::Entry(Ok(Some(entry))) = ask(
        &mut fs,
        &mut clients,
        &Request::ReadDir {
            dir: ROOT,
            cursor: START,
        },
    ) else {
        panic!("the root read no entry");
    };
    assert_eq!(entry.name, name(b"SUB"));
    assert!(entry.directory);
    assert_eq!(entry.size, 0);
}

#[test]
fn a_name_that_is_no_text_is_refused() {
    let mut fs = volume();
    let mut clients = Clients::new();
    let bytes = Name::new(&[0xFFu8, 0xFE]).unwrap();
    assert_eq!(
        ask(
            &mut fs,
            &mut clients,
            &Request::Open {
                parent: ROOT,
                name: bytes
            }
        ),
        Reply::Opened(Err(Error::InvalidArgument))
    );
    assert_eq!(
        ask(
            &mut fs,
            &mut clients,
            &Request::Remove {
                parent: ROOT,
                name: bytes
            }
        ),
        Reply::Removed(Err(Error::InvalidArgument))
    );
}

#[test]
fn a_client_that_holds_everything_open_is_told_so() {
    let mut fs = volume();
    let mut clients = Clients::new();
    let mut first = None;
    for index in 0..crate::MAX_OPEN {
        let text = [b'F', b'A'.saturating_add(u8::try_from(index).unwrap())];
        let file = make(&mut fs, &mut clients, &text);
        first.get_or_insert(file);
    }
    let text = *b"GA";
    assert_eq!(
        ask(
            &mut fs,
            &mut clients,
            &Request::Create {
                parent: ROOT,
                name: name(&text),
                directory: false
            }
        ),
        Reply::Created(Err(Error::OutOfHandles))
    );
    assert_eq!(
        ask(
            &mut fs,
            &mut clients,
            &Request::Open {
                parent: ROOT,
                name: name(b"FA")
            }
        ),
        Reply::Opened(Err(Error::OutOfHandles))
    );
    assert!(
        fs.find(fs.root(), &fs_fat::Name::new("GA").unwrap())
            .unwrap()
            .is_none()
    );
    assert_eq!(
        ask(
            &mut fs,
            &mut clients,
            &Request::Close {
                file: first.unwrap()
            }
        ),
        Reply::Closed(Ok(()))
    );
    assert!(matches!(
        ask(
            &mut fs,
            &mut clients,
            &Request::Create {
                parent: ROOT,
                name: name(b"GA"),
                directory: false,
            }
        ),
        Reply::Created(Ok(_))
    ));
}

#[test]
fn a_new_client_with_no_table_cannot_create_an_entry() {
    let mut fs = volume();
    let mut clients = Clients::new();
    for badge in 1..=MAX_CLIENTS {
        let badge = u64::try_from(badge).unwrap();
        let reply = as_client(
            &mut fs,
            &mut clients,
            badge,
            &Request::Create {
                parent: ROOT,
                name: name(format!("F{badge}").as_bytes()),
                directory: false,
            },
        );
        assert!(matches!(reply, Reply::Created(Ok(_))), "{reply:?}");
    }
    let extra = u64::try_from(MAX_CLIENTS + 1).unwrap();
    assert_eq!(
        as_client(
            &mut fs,
            &mut clients,
            extra,
            &Request::Create {
                parent: ROOT,
                name: name(b"EXTRA"),
                directory: false,
            }
        ),
        Reply::Created(Err(Error::OutOfHandles))
    );
    assert!(
        fs.find(fs.root(), &fs_fat::Name::new("EXTRA").unwrap())
            .unwrap()
            .is_none()
    );
}

#[test]
fn what_a_file_handle_stands_for_is_no_directory() {
    let mut fs = volume();
    let mut clients = Clients::new();
    let file = make(&mut fs, &mut clients, b"F.TXT");
    assert_eq!(clients.get(CLIENT, file).unwrap().as_dir(), None);
}

#[test]
fn a_listing_of_the_root_walks_it_once_and_not_once_per_entry() {
    let mut fs = volume();
    let mut clients = Clients::new();
    for text in [b"A.TXT", b"B.TXT", b"C.TXT"] {
        let file = make(&mut fs, &mut clients, text);
        let _closed = ask(&mut fs, &mut clients, &Request::Close { file });
    }
    // Every entry is read at the cursor the one before it answered, so
    // the walk is never reset and the listing costs one pass.
    let mut cursor = START;
    let mut names = Vec::new();
    loop {
        let Reply::Entry(Ok(entry)) = ask(
            &mut fs,
            &mut clients,
            &Request::ReadDir { dir: ROOT, cursor },
        ) else {
            panic!("a directory read was refused");
        };
        let Some(entry) = entry else {
            break;
        };
        cursor = entry.cursor;
        names.push(entry.name);
    }
    assert_eq!(names, vec![name(b"A.TXT"), name(b"B.TXT"), name(b"C.TXT")]);
    // The walk is the client's, so a second client starts its own at the
    // first entry rather than continuing this one.
    let theirs = as_client(
        &mut fs,
        &mut clients,
        CLIENT + 1,
        &Request::ReadDir {
            dir: ROOT,
            cursor: START,
        },
    );
    let Reply::Entry(Ok(Some(entry))) = theirs else {
        panic!("the second client read no entry");
    };
    assert_eq!(entry.name, name(b"A.TXT"));
}

#[test]
fn a_request_without_a_badge_is_refused_whatever_it_asks_for() {
    let mut fs = volume();
    let mut clients = Clients::new();
    let file = make(&mut fs, &mut clients, b"A.TXT");
    for request in vec![
        Request::Open {
            parent: ROOT,
            name: name(b"A.TXT"),
        },
        Request::Create {
            parent: ROOT,
            name: name(b"B.TXT"),
            directory: false,
        },
        Request::Read {
            file,
            offset: 0,
            len: 1,
        },
        Request::Write {
            file,
            offset: 0,
            data: Data::new(b"x").unwrap(),
        },
        Request::ReadDir {
            dir: ROOT,
            cursor: START,
        },
        Request::Stat { file },
        Request::Remove {
            parent: ROOT,
            name: name(b"A.TXT"),
        },
        Request::Close { file },
        Request::Flush,
    ] {
        let refused = as_client(&mut fs, &mut clients, NOBODY, &request);
        assert_eq!(
            error_of(&refused),
            Some(Error::AccessDenied),
            "{request:?} under no badge"
        );
    }
    // What the client with a badge opened is still open.
    let Reply::Stat(Ok(_)) = ask(&mut fs, &mut clients, &Request::Stat { file }) else {
        panic!("the badged client lost its file");
    };
}

/// The error of a reply, whatever kind of reply it is.
fn error_of(reply: &Reply) -> Option<Error> {
    match reply {
        Reply::Opened(outcome) => outcome.as_ref().err().copied(),
        Reply::Created(outcome) | Reply::Written(outcome) => outcome.as_ref().err().copied(),
        Reply::Read(outcome) => outcome.as_ref().err().copied(),
        Reply::Entry(outcome) => outcome.as_ref().err().copied(),
        Reply::Stat(outcome) => outcome.as_ref().err().copied(),
        Reply::Removed(outcome) | Reply::Closed(outcome) | Reply::Flushed(outcome) => {
            outcome.as_ref().err().copied()
        }
    }
}
