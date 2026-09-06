// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

//! The loop every server of this system runs.
//!
//! Wait for a message, answer it, wait for the next: `ipc_reply_recv` does
//! the last two in one call, which is what a server does every time round
//! except the first. The badge of the capability the sender used is the
//! only thing about it the kernel guarantees, so it is the client's name
//! here and everywhere.
//!
//! Invariant: a server answers every call it received and never twice; a
//! message that arrived through `send` rather than `call` carries no reply
//! object and is answered by not answering.

use audhsos_abi::Error;
use user_rt::{EndpointHandle, ReplyHandle};
use user_sys_x86_64::{Gate, Received};

/// What a server is holding between two messages.
#[derive(Clone, Copy, Debug, Default)]
pub struct Serving {
    /// The call that has not been answered yet, if the last message was
    /// one.
    pub pending: Option<ReplyHandle>,
    /// The badge of whoever sent the message that is being handled.
    pub badge: u64,
}

/// Waits for the next message, answering the one before it when there is
/// one to answer.
///
/// The message stands in the buffer of the gate when this returns.
///
/// # Errors
///
/// Whatever the kernel answered. A server that cannot receive has nothing
/// left to do and says so to whoever started it.
pub fn receive(
    gate: &mut Gate,
    endpoint: EndpointHandle,
    serving: &mut Serving,
) -> Result<(), Error> {
    let received: Received = match serving.pending.take() {
        Some(reply) => gate.ipc_reply_recv(reply, endpoint)?,
        None => gate.ipc_recv(endpoint)?,
    };
    serving.pending = received.reply;
    serving.badge = received.badge;
    Ok(())
}

/// Answers the call that is standing, with the message that is already in
/// the buffer.
///
/// This is for the answer a server cannot fold into its next receive — the
/// last one before it stops, and the one it sends when the message it got
/// was not a call it wants to keep waiting on.
///
/// # Errors
///
/// Whatever the kernel answered.
pub fn reply(gate: &mut Gate, serving: &mut Serving) -> Result<(), Error> {
    match serving.pending.take() {
        Some(reply) => gate.ipc_reply(reply),
        None => Ok(()),
    }
}
