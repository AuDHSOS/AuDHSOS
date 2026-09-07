// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

//! The host tests of the crate: one module for the pieces a stream is
//! made of, one for what this crate writes and reads back, and one for
//! streams another compressor wrote.

mod format;
mod roundtrip;
mod vectors;
