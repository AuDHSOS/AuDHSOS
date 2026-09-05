// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

//! The host tests of the crate: one module for the blocks, one for what
//! runs inside them, and one for the documents of this repository, which
//! are the only input the parser has to be right about.

mod block;
mod inline;
mod repository;
