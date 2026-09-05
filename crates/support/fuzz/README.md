# fuzz-support

The fuzzing engine of this project, and what every fuzz target shares: the
`fuzz_target!` macro that writes a target's two entry points, the engine
that mutates and runs, and the corpus replay that runs the stored inputs
again without a fuzzer.

A target built with the coverage instrumentation and `--cfg fuzzing` is a
fuzzer; `cargo xtask fuzz` sets those flags. The same source built without
them is an ordinary program that reads the files and directories named on
its command line and runs the body over each of them, which is how
`cargo xtask fuzz --regression` replays a corpus.

Nothing outside this workspace is linked in. The compiler emits the
counters and the comparison callbacks, `sancov` and `counters` receive
them, and everything above that is this crate's own code: the mutator, the
dictionary of values the target was seen comparing against, the corpus and
its feature bookkeeping, and the loop. This is why a machine whose clang
carries no libFuzzer runtime, which is every machine with Apple's clang,
can still fuzz.

## What it is a port of

The engine follows libFuzzer, part of the LLVM Project, closely: the
mutations and how one is drawn, the bucketing of counters into features,
the table of recent compares, the way a feature belongs to the smallest
input that reaches it, the value profile, the merge, and the names of the
command line flags. Those files name both licences in their header, and
`NOTICE` at the root of this repository carries the full notice.

Three things are deliberately not libFuzzer's:

- The value profile keys a comparison on the constant the compiler knew,
  where libFuzzer keys it on the address of the comparison. Rust hands out
  no return address, and for the comparisons a parser is made of, against
  a tag or a length, the constant tells two sites apart about as well.
- A corpus file is named by a fast hash, not by a SHA-1 digest. The name
  only has to be one name per input.
- There is no fork mode, no leak detection, no memory limit, and no
  `merge_control_file`. A run asked for one of those stops and says so.

## The unsafe

Two functions of `counters` turn the ranges the compiler registers into
slices, and the callbacks in `sancov` take the values the compiler passes.
That is all of it, and it is all in those two modules. A fuzz target above
them is safe code, and so is the macro that writes it.
