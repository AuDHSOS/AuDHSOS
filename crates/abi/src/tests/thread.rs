// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

//! Tests of `crate::thread`.

use crate::Error;
use crate::thread::{Fault, FaultKind, ThreadState};
use std::collections::HashSet;

#[test]
fn every_state_round_trips_through_its_code() {
    for &state in ThreadState::ALL {
        assert_eq!(ThreadState::from_code(state.code()), Some(state));
        assert_eq!(ThreadState::try_from(state.code()), Ok(state));
    }
}

#[test]
fn state_codes_are_unique_dense_and_non_zero() {
    let mut codes: Vec<u32> = ThreadState::ALL.iter().map(|s| s.code()).collect();
    let unique: HashSet<u32> = codes.iter().copied().collect();
    assert_eq!(unique.len(), ThreadState::ALL.len());
    codes.sort_unstable();
    let expected: Vec<u32> = (1..=u32::try_from(ThreadState::ALL.len()).unwrap()).collect();
    assert_eq!(codes, expected);
    assert_eq!(ThreadState::from_code(0), None);
    assert_eq!(ThreadState::try_from(0), Err(Error::InvalidArgument));
}

#[test]
fn the_table_holds_every_state_of_the_architecture() {
    assert_eq!(ThreadState::ALL.len(), 10);
    let names: Vec<&str> = ThreadState::ALL.iter().map(|s| s.name()).collect();
    assert_eq!(
        names,
        [
            "Inactive",
            "Ready",
            "Running",
            "BlockedSend",
            "BlockedRecv",
            "BlockedReply",
            "BlockedNotification",
            "Suspended",
            "Faulted",
            "Exited",
        ]
    );
}

#[test]
fn an_unknown_state_code_is_rejected() {
    let highest = ThreadState::ALL.iter().map(|s| s.code()).max().unwrap();
    assert_eq!(ThreadState::from_code(highest + 1), None);
    assert_eq!(ThreadState::from_code(u32::MAX), None);
}

#[test]
fn exactly_the_blocked_states_are_blocked() {
    let blocked: Vec<&str> = ThreadState::ALL
        .iter()
        .filter(|s| s.is_blocked())
        .map(|s| s.name())
        .collect();
    assert_eq!(
        blocked,
        [
            "BlockedSend",
            "BlockedRecv",
            "BlockedReply",
            "BlockedNotification"
        ]
    );
}

#[test]
fn a_state_is_never_both_runnable_and_blocked() {
    for &state in ThreadState::ALL {
        assert!(
            !(state.is_runnable() && state.is_blocked()),
            "{}",
            state.name()
        );
        if state.is_dead() {
            assert!(!state.is_runnable(), "{}", state.name());
            assert!(!state.is_blocked(), "{}", state.name());
        }
    }
    assert!(ThreadState::Ready.is_runnable());
    assert!(ThreadState::Running.is_runnable());
    assert!(!ThreadState::Inactive.is_runnable());
    assert!(!ThreadState::Suspended.is_runnable());
    assert!(!ThreadState::Faulted.is_runnable());
    assert!(ThreadState::Exited.is_dead());
    assert_eq!(
        ThreadState::ALL.iter().filter(|s| s.is_dead()).count(),
        1,
        "exactly one state is final"
    );
}

#[test]
fn every_fault_kind_round_trips_through_its_code() {
    for &kind in FaultKind::ALL {
        assert_eq!(FaultKind::from_code(kind.code()), Some(kind));
        assert_eq!(FaultKind::try_from(kind.code()), Ok(kind));
    }
}

#[test]
fn fault_codes_are_unique_dense_and_non_zero() {
    let mut codes: Vec<u32> = FaultKind::ALL.iter().map(|k| k.code()).collect();
    let unique: HashSet<u32> = codes.iter().copied().collect();
    assert_eq!(unique.len(), FaultKind::ALL.len());
    codes.sort_unstable();
    let expected: Vec<u32> = (1..=u32::try_from(FaultKind::ALL.len()).unwrap()).collect();
    assert_eq!(codes, expected);
    assert_eq!(FaultKind::from_code(0), None);
    assert_eq!(FaultKind::try_from(0), Err(Error::InvalidArgument));
    assert_eq!(FaultKind::from_code(u32::MAX), None);
}

#[test]
fn the_table_holds_every_fault_the_architecture_names() {
    let names: Vec<&str> = FaultKind::ALL.iter().map(|k| k.name()).collect();
    assert_eq!(
        names,
        [
            "PageFault",
            "GeneralProtection",
            "InvalidOpcode",
            "DivideError",
            "Breakpoint",
            "AlignmentCheck",
        ]
    );
}

#[test]
fn a_fault_carries_what_the_handler_needs() {
    let fault = Fault {
        kind: FaultKind::PageFault,
        address: 0xDEAD_BEEF,
        instruction_pointer: 0x40_0000,
        error_code: 0x7,
    };
    assert_eq!(fault.kind, FaultKind::PageFault);
    assert_eq!(fault.address, 0xDEAD_BEEF);
    assert_eq!(fault.instruction_pointer, 0x40_0000);
    assert_eq!(fault.error_code, 0x7);
    assert_eq!(fault, fault);
}
