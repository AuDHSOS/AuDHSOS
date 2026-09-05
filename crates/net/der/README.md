# audhsos-der

A reader for the distinguished encoding rules, in the strict reading a
certificate parser needs: one encoding per value, and everything else
rejected.

Strict means definite lengths only, the short form wherever it fits, no
leading zero in a length or in an integer, no high tag numbers, no
constructed string types, no indefinite forms, and no trailing bytes after
the outermost value. A certificate that a permissive parser and a strict
one read differently is a certificate two systems disagree about, which is
the shape most of the interesting attacks on this format have taken.

The reader borrows. A value is a slice into the input, so parsing a
certificate allocates nothing and copies nothing; the lifetime keeps the
result tied to the bytes it came from.

Nesting is bounded at `MAX_DEPTH`, so a deeply nested input cannot drive
the parser past what the stack allows, and every reader is finished
explicitly, so a caller cannot silently ignore what it did not read.

## Where time comes from

`UTCTime` and `GeneralizedTime` are parsed here and validated in
`audhsos-time`. This crate owns the syntax — the form RFC 5280 allows, the
digits, the `Z` suffix, the two-digit year window — and hands the six
fields to `CivilTime`, which owns what a field may hold, the true length
of a month included (D-46). A caller therefore gets a value that the
calendar accepts, converts to a `UnixTime`, and compares against a
network timer as one type.
