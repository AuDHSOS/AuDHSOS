// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

//! Tests of `crate::transition`, covering the transition item of the
//! catalog 6.6.7.

use std::collections::HashSet;

use audhsos_abi::{Error, ThreadState};

use crate::transition::{Event, TRANSITIONS, is_legal, next};

#[test]
fn every_pair_of_state_and_event_is_a_transition_or_an_error() {
    // The exhaustive test the catalog asks for: every one of the ten
    // states against every one of the thirteen events.
    for &state in ThreadState::ALL {
        for &event in Event::ALL {
            match next(state, event) {
                Ok(to) => assert!(
                    TRANSITIONS.contains(&(state, event, to)),
                    "{} + {} landed in {} without a row",
                    state.name(),
                    event.name(),
                    to.name()
                ),
                Err(error) => assert_eq!(
                    error,
                    Error::InvalidState,
                    "{} + {} failed with the wrong error",
                    state.name(),
                    event.name()
                ),
            }
        }
    }
}

#[test]
fn no_pair_appears_twice_in_the_table() {
    let mut seen = HashSet::new();
    for &(from, event, _) in TRANSITIONS {
        assert!(
            seen.insert((from, event)),
            "{} + {} is in the table twice",
            from.name(),
            event.name()
        );
    }
    assert_eq!(seen.len(), TRANSITIONS.len());
}

#[test]
fn a_thread_that_exited_does_nothing_else() {
    for &event in Event::ALL {
        assert_eq!(
            next(ThreadState::Exited, event),
            Err(Error::InvalidState),
            "{}",
            event.name()
        );
    }
    assert!(
        !TRANSITIONS
            .iter()
            .any(|(from, _, _)| *from == ThreadState::Exited)
    );
}

#[test]
fn every_state_but_the_final_one_can_be_ended() {
    for &state in ThreadState::ALL {
        if state == ThreadState::Exited {
            continue;
        }
        assert_eq!(
            next(state, Event::Exit),
            Ok(ThreadState::Exited),
            "{}",
            state.name()
        );
    }
}

#[test]
fn a_thread_starts_once_and_only_from_inactive() {
    assert_eq!(
        next(ThreadState::Inactive, Event::Start),
        Ok(ThreadState::Ready)
    );
    for &state in ThreadState::ALL {
        if state == ThreadState::Inactive {
            continue;
        }
        assert!(!is_legal(state, Event::Start), "{}", state.name());
    }
}

#[test]
fn only_the_running_thread_blocks_and_every_block_has_its_state() {
    for &event in Event::ALL {
        let Some(blocked) = event.blocked_state() else {
            continue;
        };
        assert_eq!(next(ThreadState::Running, event), Ok(blocked));
        for &state in ThreadState::ALL {
            if state == ThreadState::Running {
                continue;
            }
            assert!(
                !is_legal(state, event),
                "{} + {}",
                state.name(),
                event.name()
            );
        }
    }
    assert_eq!(
        Event::ALL
            .iter()
            .filter(|event| event.blocked_state().is_some())
            .count(),
        4
    );
}

#[test]
fn every_blocked_state_wakes_into_ready() {
    for &state in ThreadState::ALL {
        if !state.is_blocked() {
            assert!(!is_legal(state, Event::Wake), "{}", state.name());
            continue;
        }
        assert_eq!(next(state, Event::Wake), Ok(ThreadState::Ready));
    }
}

#[test]
fn only_a_stopped_thread_resumes() {
    for &state in ThreadState::ALL {
        let expected = matches!(state, ThreadState::Suspended | ThreadState::Faulted);
        assert_eq!(is_legal(state, Event::Resume), expected, "{}", state.name());
    }
    assert_eq!(
        next(ThreadState::Suspended, Event::Resume),
        Ok(ThreadState::Ready)
    );
    assert_eq!(
        next(ThreadState::Faulted, Event::Resume),
        Ok(ThreadState::Ready)
    );
}

#[test]
fn only_the_running_thread_is_preempted_yields_or_faults() {
    for event in [Event::Preempt, Event::Yield, Event::Fault] {
        for &state in ThreadState::ALL {
            let expected = state == ThreadState::Running;
            assert_eq!(
                is_legal(state, event),
                expected,
                "{} + {}",
                state.name(),
                event.name()
            );
        }
    }
    assert_eq!(
        next(ThreadState::Running, Event::Preempt),
        Ok(ThreadState::Ready)
    );
    assert_eq!(
        next(ThreadState::Running, Event::Yield),
        Ok(ThreadState::Ready)
    );
    assert_eq!(
        next(ThreadState::Running, Event::Fault),
        Ok(ThreadState::Faulted)
    );
}

#[test]
fn only_a_ready_thread_is_scheduled() {
    for &state in ThreadState::ALL {
        let expected = state == ThreadState::Ready;
        assert_eq!(
            is_legal(state, Event::Schedule),
            expected,
            "{}",
            state.name()
        );
    }
}

#[test]
fn a_thread_that_is_not_dead_can_be_suspended() {
    for &state in ThreadState::ALL {
        let expected = !matches!(
            state,
            ThreadState::Exited | ThreadState::Suspended | ThreadState::Faulted
        );
        assert_eq!(
            is_legal(state, Event::Suspend),
            expected,
            "{}",
            state.name()
        );
    }
}

#[test]
fn every_event_has_a_name_and_they_are_unique() {
    let names: HashSet<&str> = Event::ALL.iter().map(|event| event.name()).collect();
    assert_eq!(names.len(), Event::ALL.len());
    assert_eq!(Event::ALL.len(), 13);
}
