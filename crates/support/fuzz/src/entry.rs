// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

//! The macro that writes a fuzz target's entry points.
//!
//! A target says what one input does, and gets two programs from it: with
//! the instrumentation, one that fuzzes; without it, one that replays the
//! files named on its command line. Neither of them is unsafe code, and
//! neither of them names a symbol of the C world: this project's engine is
//! called from Rust, so the pointer and length that libFuzzer's entry
//! point takes never come into it.

/// Writes the entry points of a fuzz target.
///
/// The body is given the input as a `&[u8]` and may do anything with it
/// except return a value. A target that panics has found something.
#[macro_export]
macro_rules! fuzz_target {
    (|$input:ident: &[u8]| $body:block) => {
        /// What the fuzzer does with one input.
        fn test_one_input($input: &[u8]) $body

        /// Fuzzes, because this target was built with the instrumentation.
        #[cfg(fuzzing)]
        fn main() -> ::std::process::ExitCode {
            $crate::engine::run(
                ::std::env::args_os().skip(1),
                &mut test_one_input,
            )
        }

        /// Replays the corpus files named on the command line, because
        /// this target was built without the instrumentation.
        #[cfg(not(fuzzing))]
        fn main() -> ::std::process::ExitCode {
            $crate::replay_args(::std::env::args_os().skip(1), test_one_input)
        }
    };
}
