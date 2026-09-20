// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

//! Ownership of capabilities received in an input request. The snapshot
//! survives syscalls overwriting the IPC buffer. Only a successful subscribe
//! takes handles; every other path closes all of them, including malformed
//! messages and extra handles. No received capability is silently dropped.

use audhsos_abi::{Buffer, Handle};
use user_proto::handles::Carried;
use user_proto::input::{Reply, Request};

/// The handles installed by the kernel for one received message.
#[derive(Debug)]
pub struct ReceivedHandles(Carried);

impl ReceivedHandles {
    /// Copies exactly the received handle area, before any nested IPC.
    #[must_use]
    pub fn read(buffer: Buffer<'_>) -> Self {
        Self(Carried::read(buffer))
    }

    /// Closes everything except the two handles a successful subscribe took.
    pub fn finish(self, request: Option<Request>, reply: Reply, close: impl FnMut(Handle)) {
        match (request, reply) {
            (
                Some(Request::Subscribe {
                    notification,
                    process,
                }),
                Reply::Subscribed(Ok(_)),
            ) => self.0.give_up(&[notification, process], close),
            _ => self.0.give_up(&[], close),
        }
    }
}
