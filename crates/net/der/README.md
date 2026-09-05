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

## What is missing, and why

`UTCTime` and `GeneralizedTime` are parsed and checked here as far as
their syntax goes: the form RFC 5280 allows, the digits, the field ranges,
the `Z` suffix, the two-digit year window. They are *not* converted to a
point in time, and the day is checked against thirty-one rather than
against the true length of its month.

Both belong to `audhsos-time` under decision D-46, which does not exist
yet. When it does, this crate gains that dependency, `Timestamp` becomes
its `CivilTime`, and the conversion and the calendar checks arrive with
it. Until then a caller comparing certificate validity has fields, not
instants. Document 11, section 11.14, lists this and the other seams the
track is waiting on.
