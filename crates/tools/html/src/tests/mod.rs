// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

//! The host tests of the crate: one module for the tokenizer and the tree
//! it builds, one for the blocks that come out of it, and one for the
//! document of this repository, which is the only input the parser has to
//! be right about.

mod block;
mod repository;
mod syntax;
