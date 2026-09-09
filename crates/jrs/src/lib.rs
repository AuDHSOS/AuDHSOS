// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

#![cfg_attr(not(test), no_std)]
#![forbid(unsafe_code)]
#![doc = include_str!("../README.md")]

extern crate alloc;

mod bytecode;
mod error;
mod event;
mod heap;
mod iterator;
mod lexer;
mod number;
mod object;
mod parser;
mod promise;
mod regexp;
mod symbol;
mod value;
mod vm;
mod weakmap;

pub use bytecode::{Program, compile};
pub use bytecode::{Script, compile_script};
pub use error::Error;
pub use symbol::SymbolValue;
pub use value::{FunctionValue, ObjectValue, Value};
pub use vm::{Host, Realm, Runtime, SilentHost};

/// Explicit limits for untrusted source and execution.
///
/// These bound logical resources, not the allocator's total memory usage.
/// An embedding must provide an allocator and its own memory quota.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Limits {
    /// Maximum UTF-8 source length in bytes.
    pub source_bytes: usize,
    /// Maximum lexer tokens (including end of input).
    pub tokens: usize,
    /// Maximum recursive parser/compiler depth; capped internally at 48.
    pub nesting: usize,
    /// Maximum bytecode instructions in a program.
    pub instructions: usize,
    /// Maximum instructions executed per run, including back edges.
    pub fuel: u64,
    /// Maximum number of operand stack entries.
    pub stack: usize,
    /// Maximum length of a string in UTF-16 code units.
    pub string_units: usize,
    /// Maximum live binding cells and function objects in the tracing heap.
    pub heap_entries: usize,
    /// Maximum simultaneously active JavaScript calls (not Rust recursion).
    pub call_frames: usize,
    /// Maximum binding slots across all active call frames, including the script.
    pub binding_slots: usize,
    /// Maximum own properties per object, including non-enumerable properties.
    pub properties: usize,
    /// Maximum queued Promise jobs and reactions attached to any one Promise.
    pub jobs: usize,
    /// Maximum total `WeakMap` entries across the execution; reclaimed by GC.
    pub weak_entries: usize,
}

impl Default for Limits {
    fn default() -> Self {
        Self {
            source_bytes: 1_048_576,
            tokens: 262_144,
            nesting: 128,
            instructions: 1_048_576,
            fuel: 10_000_000,
            stack: 4096,
            string_units: 1_048_576,
            heap_entries: 65_536,
            call_frames: 1024,
            binding_slots: 65_536,
            properties: 65_536,
            jobs: 65_536,
            weak_entries: 65_536,
        }
    }
}

#[cfg(test)]
mod tests;
