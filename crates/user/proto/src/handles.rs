// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

//! The capabilities a received message carried.
//!
//! The kernel installs up to [`MAX_MESSAGE_HANDLES`] handles in the
//! receiver's table before the receiver sees the message, whatever the
//! protocol says a request carries. A server that leaves one installed
//! holds a reference the sender chose and loses a slot of a table that is
//! finite, so every handle a request did not keep is given up before the
//! next receive.
//!
//! The snapshot is read before any other call of the program, because a
//! call overwrites the IPC buffer the handle area lies in.

use audhsos_abi::layout::MAX_MESSAGE_HANDLES;
use audhsos_abi::{Buffer, Handle};

/// The handles the kernel installed for one received message.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub struct Carried {
    held: [Option<Handle>; MAX_MESSAGE_HANDLES],
}

impl Carried {
    /// Copies the handle area of the message that stands in `buffer`.
    ///
    /// A buffer that holds no readable message carries no handles.
    #[must_use]
    pub fn read(buffer: Buffer<'_>) -> Self {
        let mut held = [None; MAX_MESSAGE_HANDLES];
        let Ok(message) = buffer.message() else {
            return Carried { held };
        };
        for (slot, index) in held.iter_mut().zip(0..message.handle_count) {
            *slot = buffer.handle(index);
        }
        Carried { held }
    }

    /// Every handle the message carried, in the order it carried them.
    pub fn handles(&self) -> impl Iterator<Item = Handle> + '_ {
        self.held.iter().flatten().copied()
    }

    /// How many handles the message carried.
    #[must_use]
    pub fn count(&self) -> usize {
        self.handles().count()
    }

    /// Gives up every handle except the ones in `kept`, which the request
    /// took.
    pub fn give_up(&self, kept: &[Handle], mut close: impl FnMut(Handle)) {
        for handle in self.handles() {
            if !kept.contains(&handle) {
                close(handle);
            }
        }
    }
}
