// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

//! The debug console.

/// Writes diagnostic bytes; a no-op in builds without the debug console.
pub trait DebugConsole {
    /// Writes every byte in order.
    fn write_bytes(&mut self, bytes: &[u8]);
}

/// A mutable reference to a console is a console, so that the kernel can
/// hand one out without giving it away.
impl<T: DebugConsole + ?Sized> DebugConsole for &mut T {
    fn write_bytes(&mut self, bytes: &[u8]) {
        (**self).write_bytes(bytes);
    }
}
