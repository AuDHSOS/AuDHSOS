// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

//! Contention, recursive refusal, and publication across processor owners.
use crate::{Error, ExclusiveToken, Global, Preset};
use std::sync::atomic::{AtomicBool, Ordering};

struct Token<'a>(u32, &'a AtomicBool);
impl ExclusiveToken for Token<'_> {
    fn owner(&self) -> u32 {
        self.0
    }
    fn wait(&self) {
        self.1.store(true, Ordering::Release);
        std::thread::yield_now();
    }
}

#[test]
fn a_different_owner_waits_and_observes_the_released_value() {
    let waited = AtomicBool::new(false);
    let global = Global::new();
    global.init(0u32).unwrap();
    let preset = Preset::new(0u32);
    std::thread::scope(|scope| {
        let mut first = global.borrow(&Token(0, &waited)).unwrap();
        let mut second = preset.borrow(&Token(0, &waited)).unwrap();
        let thread = scope.spawn(|| {
            let mut global = global.borrow(&Token(1, &waited)).unwrap();
            assert_eq!(*global, 41);
            *global += 1;
            assert_eq!(*preset.borrow(&Token(1, &waited)).unwrap(), 42);
        });
        while !waited.load(Ordering::Acquire) {
            std::thread::yield_now();
        }
        *first = 41;
        *second = 42;
        drop(second);
        drop(first);
        thread.join().unwrap();
    });
    assert_eq!(*global.borrow(&Token(0, &waited)).unwrap(), 42);
}

#[test]
fn a_matching_owner_on_another_thread_is_refused_without_waiting() {
    let waited = AtomicBool::new(false);
    let preset = Preset::new(7);
    let global = Global::new();
    global.init(9).unwrap();
    let _first = preset.borrow(&Token(4, &waited)).unwrap();
    let _second = global.borrow(&Token(4, &waited)).unwrap();
    std::thread::scope(|scope| {
        scope
            .spawn(|| {
                assert_eq!(
                    preset.borrow(&Token(4, &waited)).unwrap_err(),
                    Error::AlreadyBorrowed
                );
                assert_eq!(
                    global.borrow(&Token(4, &waited)).unwrap_err(),
                    Error::AlreadyBorrowed
                );
            })
            .join()
            .unwrap();
    });
    assert!(!waited.load(Ordering::Relaxed));
}

#[test]
fn the_reserved_owner_cannot_acquire_a_cell() {
    let waited = AtomicBool::new(false);
    let cell = Preset::new(1);
    assert_eq!(
        cell.borrow(&Token(u32::MAX, &waited)).unwrap_err(),
        Error::AlreadyBorrowed
    );
    assert!(!cell.is_borrowed());
}

#[test]
fn preset_contention_polls_until_release() {
    let waited = AtomicBool::new(false);
    let cell = Preset::new(0);
    std::thread::scope(|scope| {
        let mut held = cell.borrow(&Token(0, &waited)).unwrap();
        let other = scope.spawn(|| assert_eq!(*cell.borrow(&Token(1, &waited)).unwrap(), 23));
        while !waited.load(Ordering::Acquire) {
            std::thread::yield_now();
        }
        *held = 23;
        drop(held);
        other.join().unwrap();
    });
    let global = Global::<u32>::new();
    assert_eq!(
        global.borrow(&Token(u32::MAX, &waited)).unwrap_err(),
        Error::AlreadyBorrowed
    );
    assert_eq!(
        global.borrow(&Token(1, &waited)).unwrap_err(),
        Error::Uninitialized
    );
    crate::UncontendedToken.wait();
}
