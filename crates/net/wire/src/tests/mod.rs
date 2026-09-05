// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

//! The host tests of the crate, one module per product module, except
//! that the addresses take three: `addr` holds the hardware address, the
//! port, and the types that carry either family, and `ipv4` and `ipv6`
//! hold one family each. They are the largest part of the crate and the
//! two families answer to different documents.

mod addr;
mod checksum;
mod cursor;
mod error;
mod ipv4;
mod ipv6;
mod protocol;
