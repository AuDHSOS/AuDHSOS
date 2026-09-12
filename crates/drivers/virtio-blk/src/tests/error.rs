// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

//! Tests of `crate::error`.

use virtio_queue::{DeviceError, QueueError};

use crate::error::BlkError;

/// One of every refusal.
const EVERY: [BlkError; 13] = [
    BlkError::NotReset(7),
    BlkError::NoQueue,
    BlkError::QueueSize(3),
    BlkError::NoVector(1),
    BlkError::Generation(8),
    BlkError::Capacity {
        sector: 2047,
        sectors: 4,
        capacity: 2048,
    },
    BlkError::DataLength(513),
    BlkError::Framing,
    BlkError::FlushSector(9),
    BlkError::ReadOnly,
    BlkError::Buffer(15),
    BlkError::Status(1),
    BlkError::Device(DeviceError::Legacy),
];

#[test]
fn every_refusal_renders_a_sentence_of_its_own() {
    let mut rendered: Vec<String> = EVERY.iter().map(ToString::to_string).collect();
    rendered.push(BlkError::Queue(QueueError::NotLive).to_string());
    assert!(rendered.iter().all(|text| !text.is_empty()));
    rendered.sort();
    let count = rendered.len();
    rendered.dedup();
    assert_eq!(rendered.len(), count, "two refusals read the same");
}

#[test]
fn the_errors_of_the_queue_crate_come_through_as_they_are() {
    assert_eq!(
        BlkError::from(DeviceError::Legacy),
        BlkError::Device(DeviceError::Legacy)
    );
    assert_eq!(
        BlkError::from(QueueError::NotLive),
        BlkError::Queue(QueueError::NotLive)
    );
    assert_eq!(
        BlkError::Device(DeviceError::Legacy).to_string(),
        DeviceError::Legacy.to_string()
    );
}

#[test]
fn a_refusal_is_an_error() {
    let error: &dyn core::error::Error = &BlkError::NoQueue;
    assert!(error.source().is_none());
}
