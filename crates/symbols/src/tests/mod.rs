// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

//! Unit tests, kept out of the product sources so that coverage measures
//! product code only.

#![allow(clippy::arithmetic_side_effects, clippy::indexing_slicing)]
#![allow(clippy::as_conversions, clippy::missing_panics_doc)]
#![allow(unreachable_pub, clippy::missing_docs_in_private_items)]
#![allow(
    clippy::cast_possible_truncation,
    clippy::needless_pass_by_value,
    clippy::manual_is_multiple_of,
    clippy::same_item_push
)]

mod build;
mod cursor;
mod functions;
mod line;
mod symbols;
