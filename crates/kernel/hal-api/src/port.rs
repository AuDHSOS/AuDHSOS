// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

//! Port I/O (`x86_64` only).

/// Reads and writes I/O ports.
pub trait PortAccess {
    /// Reads one byte.
    fn read_u8(&mut self, port: u16) -> u8;
    /// Writes one byte.
    fn write_u8(&mut self, port: u16, value: u8);
    /// Reads two bytes.
    fn read_u16(&mut self, port: u16) -> u16;
    /// Writes two bytes.
    fn write_u16(&mut self, port: u16, value: u16);
    /// Reads four bytes.
    fn read_u32(&mut self, port: u16) -> u32;
    /// Writes four bytes.
    fn write_u32(&mut self, port: u16, value: u32);
}
