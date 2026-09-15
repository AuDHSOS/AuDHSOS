// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

//! Tests of `crate::proto`.

use crate::proto::{Down, Draw, NO_FILE, Round, Up};

/// The bytes `message` writes.
fn wrote_down(message: &Down) -> Vec<u8> {
    let mut out = Vec::new();
    message.write(&mut out).unwrap_or(());
    out
}

/// The bytes `message` writes.
fn wrote_up(message: &Up) -> Vec<u8> {
    let mut out = Vec::new();
    message.write(&mut out).unwrap_or(());
    out
}

/// What reading `bytes` back as a message of the orchestrator gives.
fn read_down(bytes: &[u8]) -> std::io::Result<Down> {
    Down::read(&mut &*bytes)
}

/// What reading `bytes` back as a message of a worker gives.
fn read_up(bytes: &[u8]) -> std::io::Result<Up> {
    Up::read(&mut &*bytes)
}

#[test]
fn every_message_of_the_orchestrator_survives_the_wire() {
    let batch = Down::Batch {
        limit: 64,
        rounds: vec![
            Round {
                seed: Draw {
                    index: 3,
                    bytes: Some(b"seed".to_vec()),
                },
                cross: Draw {
                    index: 4,
                    bytes: None,
                },
            },
            Round {
                seed: Draw {
                    index: 0,
                    bytes: None,
                },
                cross: Draw {
                    index: 1,
                    bytes: Some(Vec::new()),
                },
            },
        ],
    };
    for message in [
        Down::Load(vec![0, 1, 9]),
        Down::Load(Vec::new()),
        Down::Cover(vec![1, 2, 3, 4, 5, 6, 7, 8]),
        batch,
        Down::Stop,
    ] {
        let bytes = wrote_down(&message);
        assert_eq!(read_down(&bytes).ok(), Some(message));
    }
}

#[test]
fn every_message_of_a_worker_survives_the_wire() {
    for message in [
        Up::Found {
            origin: 12,
            bytes: b"input".to_vec(),
            features: vec![7, 8, 9],
        },
        Up::Found {
            origin: NO_FILE,
            bytes: Vec::new(),
            features: Vec::new(),
        },
        Up::Idle(0),
        Up::Idle(u64::MAX),
        Up::Crash(b"boom".to_vec()),
    ] {
        let bytes = wrote_up(&message);
        assert_eq!(read_up(&bytes).ok(), Some(message));
    }
}

#[test]
fn a_kind_neither_side_writes_is_refused() {
    assert_eq!(
        read_down(&[9]).map_err(|e| e.kind()),
        Err(std::io::ErrorKind::InvalidData)
    );
    assert_eq!(
        read_up(&[9]).map_err(|e| e.kind()),
        Err(std::io::ErrorKind::InvalidData)
    );
}

#[test]
fn a_pipe_that_ended_in_the_middle_of_a_message_is_refused() {
    let bytes = wrote_down(&Down::Cover(vec![1, 2, 3, 4]));
    for cut in 0..bytes.len() {
        assert!(read_down(bytes.get(..cut).unwrap_or_default()).is_err());
    }
    let bytes = wrote_up(&Up::Idle(5));
    for cut in 0..bytes.len() {
        assert!(read_up(bytes.get(..cut).unwrap_or_default()).is_err());
    }
    assert!(
        read_up(&[
            1, 0, 0, 0, 0, // a find of no file
            4, 0, 0, 0, // four bytes
        ])
        .is_err()
    );
}

#[test]
fn a_length_no_message_has_is_refused_rather_than_allocated() {
    let mut bytes = vec![2u8];
    bytes.extend_from_slice(&u32::MAX.to_le_bytes());
    assert_eq!(
        read_down(&bytes).map_err(|e| e.kind()),
        Err(std::io::ErrorKind::InvalidData)
    );
    let mut bytes = vec![1u8];
    bytes.extend_from_slice(&u32::MAX.to_le_bytes());
    assert_eq!(
        read_down(&bytes).map_err(|e| e.kind()),
        Err(std::io::ErrorKind::InvalidData)
    );
}

#[test]
fn a_draw_is_written_by_its_place_once_the_worker_holds_it() {
    let held = wrote_down(&Down::Batch {
        limit: 1,
        rounds: vec![Round {
            seed: Draw {
                index: 0,
                bytes: None,
            },
            cross: Draw {
                index: 0,
                bytes: None,
            },
        }],
    });
    let sent = wrote_down(&Down::Batch {
        limit: 1,
        rounds: vec![Round {
            seed: Draw {
                index: 0,
                bytes: Some(vec![0; 64]),
            },
            cross: Draw {
                index: 0,
                bytes: Some(vec![0; 64]),
            },
        }],
    });
    assert_eq!(held.len().saturating_add(128), sent.len());
}
