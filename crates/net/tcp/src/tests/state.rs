// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

//! What each of the eleven states allows.

use crate::state::State;

/// The eleven, in the order RFC 9293 lists them.
const ALL: [State; 11] = [
    State::Closed,
    State::Listen,
    State::SynSent,
    State::SynReceived,
    State::Established,
    State::FinWait1,
    State::FinWait2,
    State::CloseWait,
    State::Closing,
    State::LastAck,
    State::TimeWait,
];

#[test]
fn a_state_is_synchronized_once_both_ends_have_agreed_on_numbers() {
    for state in ALL {
        let expected = !matches!(
            state,
            State::Closed | State::Listen | State::SynSent | State::SynReceived
        );
        assert_eq!(state.is_synchronized(), expected, "{state}");
    }
}

#[test]
fn a_state_carries_bytes_out_until_this_end_has_closed() {
    for state in ALL {
        let expected = matches!(state, State::Established | State::CloseWait);
        assert_eq!(state.can_send(), expected, "{state}");
    }
}

#[test]
fn a_state_carries_bytes_in_until_the_peer_has_closed() {
    for state in ALL {
        let expected = matches!(
            state,
            State::Established | State::FinWait1 | State::FinWait2
        );
        assert_eq!(state.can_receive(), expected, "{state}");
    }
}

#[test]
fn only_the_closed_state_is_the_absence_of_a_connection() {
    for state in ALL {
        assert_eq!(state.is_open(), state != State::Closed, "{state}");
    }
    assert_eq!(State::default(), State::Closed);
}

#[test]
fn every_state_writes_itself_as_the_memo_names_it() {
    let names = [
        "CLOSED",
        "LISTEN",
        "SYN-SENT",
        "SYN-RECEIVED",
        "ESTABLISHED",
        "FIN-WAIT-1",
        "FIN-WAIT-2",
        "CLOSE-WAIT",
        "CLOSING",
        "LAST-ACK",
        "TIME-WAIT",
    ];
    for (state, name) in ALL.into_iter().zip(names) {
        assert_eq!(state.to_string(), name);
    }
}
