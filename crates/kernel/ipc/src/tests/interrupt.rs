// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

//! Tests of `crate::interrupt`.

use audhsos_abi::{Error, ThreadState};
use kernel_objects::object::Interrupt;

use super::fixture::Fixture;
use crate::interrupt::{acknowledge, bind, deliver, interrupt_for, interrupt_of_line};
use crate::notify::wait;
use crate::outcome::Wakeup;

/// The vector ISA line zero reaches under the plan of the tables.
const VECTOR: u8 = 0x40;

#[test]
fn an_interrupt_signals_exactly_the_bit_it_is_bound_to_and_masks_its_line() {
    let mut fixture = Fixture::new();
    let owner = fixture.process(8);
    let driver = fixture.running(owner, 4);
    let notification = fixture.notification();
    let interrupt = fixture
        .objects
        .interrupts
        .allocate(Interrupt::new(0, VECTOR))
        .unwrap();
    bind(&mut fixture.objects, interrupt, notification, 3).unwrap();
    wait(
        &mut fixture.objects,
        &mut fixture.scheduler,
        driver,
        notification,
    )
    .unwrap();

    let outcome = deliver(&mut fixture.objects, &mut fixture.scheduler, VECTOR).unwrap();
    assert_eq!(outcome.wakeup, Some(Wakeup::ok(driver, [1 << 3, 0])));
    assert_eq!(fixture.state(driver), Some(ThreadState::Ready));
    assert!(
        fixture.objects.interrupts.get(interrupt).unwrap().masked,
        "the line is masked until the driver acknowledges"
    );
}

#[test]
fn an_interrupt_that_arrives_before_the_driver_waits_is_kept_in_the_word() {
    let mut fixture = Fixture::new();
    let owner = fixture.process(8);
    let driver = fixture.running(owner, 4);
    let notification = fixture.notification();
    let interrupt = fixture
        .objects
        .interrupts
        .allocate(Interrupt::new(0, VECTOR))
        .unwrap();
    bind(&mut fixture.objects, interrupt, notification, 5).unwrap();
    let outcome = deliver(&mut fixture.objects, &mut fixture.scheduler, VECTOR).unwrap();
    assert_eq!(outcome.wakeup, None);
    let outcome = wait(
        &mut fixture.objects,
        &mut fixture.scheduler,
        driver,
        notification,
    )
    .unwrap();
    assert_eq!(outcome.values, [1 << 5, 0]);
}

#[test]
fn an_acknowledged_interrupt_lets_the_next_one_arrive() {
    let mut fixture = Fixture::new();
    let owner = fixture.process(8);
    let driver = fixture.running(owner, 4);
    let notification = fixture.notification();
    let interrupt = fixture
        .objects
        .interrupts
        .allocate(Interrupt::new(7, VECTOR))
        .unwrap();
    bind(&mut fixture.objects, interrupt, notification, 0).unwrap();
    deliver(&mut fixture.objects, &mut fixture.scheduler, VECTOR).unwrap();
    assert_eq!(acknowledge(&mut fixture.objects, interrupt), Ok(7));
    assert!(!fixture.objects.interrupts.get(interrupt).unwrap().masked);
    deliver(&mut fixture.objects, &mut fixture.scheduler, VECTOR).unwrap();
    let outcome = wait(
        &mut fixture.objects,
        &mut fixture.scheduler,
        driver,
        notification,
    )
    .unwrap();
    assert_eq!(outcome.values, [1, 0], "the second one arrived");
}

#[test]
fn a_vector_no_interrupt_object_names_delivers_nothing() {
    let mut fixture = Fixture::new();
    assert!(deliver(&mut fixture.objects, &mut fixture.scheduler, 0x41).is_none());
    assert_eq!(interrupt_for(&fixture.objects, 0x41), None);
    fixture
        .objects
        .interrupts
        .allocate(Interrupt::new(0, VECTOR))
        .unwrap();
    assert!(deliver(&mut fixture.objects, &mut fixture.scheduler, 0x41).is_none());
}

#[test]
fn an_interrupt_bound_to_nothing_masks_its_line_and_signals_nobody() {
    let mut fixture = Fixture::new();
    let interrupt = fixture
        .objects
        .interrupts
        .allocate(Interrupt::new(0, VECTOR))
        .unwrap();
    let outcome = deliver(&mut fixture.objects, &mut fixture.scheduler, VECTOR).unwrap();
    assert_eq!(outcome.wakeup, None);
    assert!(fixture.objects.interrupts.get(interrupt).unwrap().masked);
}

#[test]
fn an_interrupt_bound_to_a_notification_that_is_gone_delivers_nothing() {
    let mut fixture = Fixture::new();
    let notification = fixture.notification();
    let interrupt = fixture
        .objects
        .interrupts
        .allocate(Interrupt::new(0, VECTOR))
        .unwrap();
    bind(&mut fixture.objects, interrupt, notification, 1).unwrap();
    fixture
        .objects
        .destroy(kernel_objects::object::AnyObjectId::of(notification))
        .expect("the last reference went");
    let outcome = deliver(&mut fixture.objects, &mut fixture.scheduler, VECTOR).unwrap();
    assert_eq!(outcome.wakeup, None);
}

#[test]
fn the_pool_says_which_object_already_names_a_line() {
    let mut fixture = Fixture::new();
    assert_eq!(interrupt_of_line(&fixture.objects, 0), None);
    let interrupt = fixture
        .objects
        .interrupts
        .allocate(Interrupt::new(0, VECTOR))
        .unwrap();
    assert_eq!(interrupt_of_line(&fixture.objects, 0), Some(interrupt));
    assert_eq!(interrupt_of_line(&fixture.objects, 1), None);
    assert_eq!(interrupt_for(&fixture.objects, VECTOR), Some(interrupt));
}

#[test]
fn a_binding_names_a_bit_of_the_word_and_no_more() {
    let mut fixture = Fixture::new();
    let notification = fixture.notification();
    let interrupt = fixture
        .objects
        .interrupts
        .allocate(Interrupt::new(0, VECTOR))
        .unwrap();
    assert_eq!(
        bind(&mut fixture.objects, interrupt, notification, 64),
        Err(Error::InvalidArgument)
    );
    assert!(bind(&mut fixture.objects, interrupt, notification, 63).is_ok());
    assert_eq!(
        fixture
            .objects
            .interrupts
            .get(interrupt)
            .unwrap()
            .notification,
        Some((notification, 63)),
        "the interrupt carries the notification and the bit, and nothing else does"
    );
    assert!(
        bind(&mut fixture.objects, interrupt, notification, 2).is_ok(),
        "the same interrupt may move its bit"
    );
}

#[test]
fn two_interrupts_bound_to_one_notification_each_set_their_own_bit() {
    let mut fixture = Fixture::new();
    let notification = fixture.notification();
    let first = fixture
        .objects
        .interrupts
        .allocate(Interrupt::new(0, VECTOR))
        .unwrap();
    let second = fixture
        .objects
        .interrupts
        .allocate(Interrupt::new(1, 0x41))
        .unwrap();
    bind(&mut fixture.objects, first, notification, 0).unwrap();
    bind(&mut fixture.objects, second, notification, 1).unwrap();

    deliver(&mut fixture.objects, &mut fixture.scheduler, VECTOR).unwrap();
    deliver(&mut fixture.objects, &mut fixture.scheduler, 0x41).unwrap();
    assert_eq!(
        fixture
            .objects
            .notifications
            .get(notification)
            .unwrap()
            .word,
        0b11,
        "the controller with two lines wakes one thread, and the word says which line"
    );
}

#[test]
fn an_object_that_is_gone_can_neither_be_bound_nor_acknowledged() {
    let mut fixture = Fixture::new();
    let notification = fixture.notification();
    let stale = kernel_objects::pool::ObjectId::new(9, 2);
    assert_eq!(
        bind(&mut fixture.objects, stale, notification, 0),
        Err(Error::InvalidHandle)
    );
    assert_eq!(
        acknowledge(&mut fixture.objects, stale),
        Err(Error::InvalidHandle)
    );
    let interrupt = fixture
        .objects
        .interrupts
        .allocate(Interrupt::new(0, VECTOR))
        .unwrap();
    let gone = kernel_objects::pool::ObjectId::new(9, 2);
    assert_eq!(
        bind(&mut fixture.objects, interrupt, gone, 0),
        Err(Error::InvalidHandle)
    );
}
