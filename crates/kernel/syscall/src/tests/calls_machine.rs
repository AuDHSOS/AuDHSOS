// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

//! Tests of `crate::calls::machine`: the clock and the entropy source, as a
//! user thread reaches them. Catalog 6.6.59 and 6.6.60.

use audhsos_abi::Error;
use audhsos_abi::Syscall;
use audhsos_abi::ipc_buffer::Buffer;

use super::double::{Call, Fixture, call, error_of, request, value_of};

#[test]
fn the_clock_answers_the_microseconds_of_the_machine() {
    let mut fixture = Fixture::new();
    fixture.environment.now = 0;
    assert_eq!(value_of(&mut fixture, request(Syscall::ClockNow, &[])), 0);
    fixture.environment.now = 50_000;
    assert_eq!(
        value_of(&mut fixture, request(Syscall::ClockNow, &[])),
        50_000
    );
}

#[test]
fn the_clock_takes_no_handle_and_needs_no_right() {
    assert_eq!(Syscall::ClockNow.argument_count(), 0);
    assert!(!Syscall::ClockNow.takes_handle());
    assert_eq!(
        crate::dispatch::required_rights(Syscall::ClockNow),
        audhsos_abi::Rights::EMPTY
    );
}

#[test]
fn a_seed_is_four_words_in_the_message_area() {
    let mut fixture = Fixture::new();
    let seed = [0x1111_1111_1111_1111, 0x2222, 0x3333, 0x4444];
    fixture.environment.seeds.push_back(Ok(seed));
    let mut buffer = request(Syscall::RandomBytes, &[]);
    let (status, values, _) = call(&mut fixture, &mut buffer);
    assert_eq!(status.error(), None);
    assert_eq!(values[0], 4, "four words, as the convention says");
    let view = Buffer::new(&buffer);
    let header = view.message().unwrap();
    assert_eq!(header.label, 0);
    assert_eq!(header.word_count, 4);
    assert_eq!(header.handle_count, 0);
    for (index, word) in seed.iter().enumerate() {
        assert_eq!(view.word(index), Some(*word));
    }
    assert_eq!(fixture.environment.count(&Call::Seed), 1);
}

#[test]
fn two_draws_are_two_calls_to_the_source() {
    let mut fixture = Fixture::new();
    fixture.environment.seeds.push_back(Ok([1, 2, 3, 4]));
    fixture.environment.seeds.push_back(Ok([5, 6, 7, 8]));
    let mut first = request(Syscall::RandomBytes, &[]);
    call(&mut fixture, &mut first);
    let mut second = request(Syscall::RandomBytes, &[]);
    call(&mut fixture, &mut second);
    assert_eq!(Buffer::new(&first).word(0), Some(1));
    assert_eq!(Buffer::new(&second).word(0), Some(5));
    assert_eq!(fixture.environment.count(&Call::Seed), 2);
}

#[test]
fn a_source_that_would_not_deliver_is_unavailable_and_writes_nothing() {
    let mut fixture = Fixture::new();
    fixture.environment.seeds.push_back(Err(Error::Unavailable));
    let mut buffer = request(Syscall::RandomBytes, &[]);
    let (status, values, _) = call(&mut fixture, &mut buffer);
    assert_eq!(status.error(), Some(Error::Unavailable));
    assert_eq!(values, [0, 0]);
    assert_eq!(Buffer::new(&buffer).message().unwrap().word_count, 0);
}

#[test]
fn a_machine_without_a_source_answers_unavailable() {
    let mut fixture = Fixture::new();
    assert_eq!(
        error_of(&mut fixture, request(Syscall::RandomBytes, &[])),
        Some(Error::Unavailable)
    );
}
