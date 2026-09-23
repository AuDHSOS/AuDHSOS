// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

//! Counts messages between fresh seeds for the network server.

/// The maximum messages served from one seed.
pub const MESSAGES_PER_SEED: u32 = 65_536;

/// The server replaces its generator before serving the next message.
#[derive(Default)]
pub struct ReseedCounter(u32);

impl ReseedCounter {
    /// Reports when the generator needs a fresh seed for this message.
    pub const fn due(&mut self) -> bool {
        if self.0 == MESSAGES_PER_SEED {
            self.0 = 1;
            true
        } else {
            self.0 = self.0.saturating_add(1);
            false
        }
    }
}
