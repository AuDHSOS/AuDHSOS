# audhsos-sync

`Global<T>`: a cell for global state that hands out exclusive access one
borrower at a time and reports a second borrower as an error instead of
waiting. This is an adapter crate on the allowlist of the safety policy; it
contains two `unsafe` sites.
