# fuzz-support

What every fuzz target of the project shares: the `LLVMFuzzerTestOneInput`
entry the fuzzer calls, the `fuzz_target!` macro that writes it once, and
the corpus replay that runs the stored inputs again without a fuzzer.

A target built with the coverage instrumentation and `--cfg fuzzing` is a
libFuzzer binary; `cargo xtask fuzz` sets those flags and links the runtime
from the platform's clang, because the workspace has no dependency outside
itself. The same source built without those flags is an ordinary program
that reads the files and directories named on its command line and runs the
body over each of them, which is how `cargo xtask fuzz --regression`
replays a corpus on a machine that has no fuzzer runtime at all.

The `unsafe` of the project's fuzzing lives here, in one function: the
fuzzer hands over a pointer and a length, and everything above this crate
sees a `&[u8]`.
