# audhsos-sync

Two cells for global state that hand out exclusive access one borrower at a
time and report a second borrower as an error instead of waiting.

`Global<T>` is written once at run time and holds an `Option<T>` until then.
`Preset<T>` holds its value from the start: no initialization step, no
`Option`, and a `const` constructor, so a `static` whose value is all zeros
reaches the `.bss` and never travels over a stack. The kernel's object pools
are of the second kind, because they are far larger than the boot stack
(D-66).

This is an adapter crate on the allowlist of the safety policy; it contains
four `unsafe` sites, two per cell.
