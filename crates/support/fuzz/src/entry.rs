// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

//! The entry the fuzzer calls, and the macro that writes it.
//!
//! Invariant: the fuzzer's pointer and length become a slice in exactly one
//! place, so that every target above this crate is safe code.

/// The input the fuzzer passes, as a slice.
///
/// A length of zero yields the empty slice without touching `data`, which
/// is what a fuzzer that passes a dangling or null pointer for an empty
/// input needs.
///
/// # Safety
///
/// For a non-zero `len`, `data` must point at `len` initialized bytes that
/// stay valid and unwritten for as long as the caller keeps the slice.
#[must_use]
pub const unsafe fn input<'a>(data: *const u8, len: usize) -> &'a [u8] {
    if len == 0 || data.is_null() {
        return &[];
    }
    // SAFETY: the caller promises `len` initialized bytes at `data` that
    // outlive the slice and that nothing writes during it; the branch above
    // ruled out the two cases `from_raw_parts` does not allow, a length of
    // zero and a null pointer.
    unsafe { core::slice::from_raw_parts(data, len) }
}

/// Writes the entry points of a fuzz target: the one the fuzzer calls, and,
/// for a build without `--cfg fuzzing`, a program that replays the files
/// and directories named on its command line.
///
/// The file that uses this macro carries `#![cfg_attr(fuzzing, no_main)]`,
/// because the fuzzer runtime brings its own `main`.
#[macro_export]
macro_rules! fuzz_target {
    (|$input:ident: &[u8]| $body:block) => {
        /// The entry the fuzzer calls once per input.
        #[unsafe(no_mangle)]
        pub extern "C" fn LLVMFuzzerTestOneInput(data: *const u8, len: usize) -> i32 {
            // SAFETY: the fuzzer passes `len` initialized bytes at `data`
            // and keeps them alive and unwritten for the call.
            let $input: &[u8] = unsafe { $crate::input(data, len) };
            $body
            0
        }

        /// Replays the corpus files named on the command line.
        #[cfg(not(fuzzing))]
        fn main() -> ::std::process::ExitCode {
            $crate::replay_args(::std::env::args_os().skip(1), |$input: &[u8]| $body)
        }
    };
}
