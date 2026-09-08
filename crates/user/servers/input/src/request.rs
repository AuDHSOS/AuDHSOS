// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

//! Ownership of capabilities received in an input request. The snapshot
//! survives syscalls overwriting the IPC buffer. Only a successful subscribe
//! takes handles; every other path closes all of them, including malformed
//! messages and extra handles. No received capability is silently dropped.

use audhsos_abi::layout::MAX_MESSAGE_HANDLES;
use audhsos_abi::{Buffer, Handle};
use audhsos_collections::ArrayVec;
use user_proto::input::{Reply, Request};

/// The handles installed by the kernel for one received message.
#[derive(Debug)]
pub struct ReceivedHandles(ArrayVec<Handle, MAX_MESSAGE_HANDLES>);

impl ReceivedHandles {
    /// Copies exactly the received handle area, before any nested IPC.
    #[must_use]
    pub fn read(buffer: Buffer<'_>) -> Self {
        let mut held = ArrayVec::new();
        if let Ok(message) = buffer.message() {
            for index in 0..message.handle_count {
                if let Some(handle) = buffer.handle(index) {
                    let _added = held.push(handle);
                }
            }
        }
        Self(held)
    }

    /// Closes everything except the two handles a successful subscribe took.
    pub fn finish(self, request: Option<Request>, reply: Reply, mut close: impl FnMut(Handle)) {
        for handle in &self.0 {
            let kept = match (request, reply) {
                (
                    Some(Request::Subscribe {
                        notification,
                        process,
                    }),
                    Reply::Subscribed(Ok(_)),
                ) => *handle == notification || *handle == process,
                _ => false,
            };
            if !kept {
                close(*handle);
            }
        }
    }
}
