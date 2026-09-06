// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

#![cfg_attr(not(test), no_std)]
#![forbid(unsafe_code)]
#![doc = include_str!("../README.md")]

pub mod cancel;
pub mod destroy;
pub mod endpoint;
pub mod interrupt;
pub mod notify;
pub mod outcome;
pub mod transfer;

pub use cancel::cancel;
pub use destroy::{destroy_endpoint, destroy_notification, destroy_reply, destroyed};
pub use endpoint::{
    Handover, Intent, Meeting, Reception, close_reply, open_reply, received, recv, replied,
    reply_caller, send, sent, undo_meeting,
};
pub use interrupt::{acknowledge, bind, deliver, interrupt_for, interrupt_of_line};
pub use notify::{poll, signal, wait};
pub use outcome::{Outcome, Waiters, Wakeup};
pub use transfer::{Transferred, transfer};

#[cfg(test)]
mod tests;
