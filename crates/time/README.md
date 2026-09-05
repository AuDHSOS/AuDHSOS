# audhsos-time

Time as arithmetic. The crate owns four types — `CivilTime`, `UnixTime`,
`Instant`, and `Duration` — and it reads no clock. A caller that knows
what time it is passes the value in; a caller that does not cannot get
one here. That is what lets the certificate validity window, the file
timestamps of FAT32, and the retransmission timers of the network stack
speak one vocabulary without any of them touching hardware (D-46).

`CivilTime` is a date and a time of day in UTC, and `UnixTime` is a count
of seconds from 1970-01-01T00:00:00Z. The conversion between them is the
integer calendar of the proleptic Gregorian rule: no floating point, no
lookup table beyond the twelve month lengths, and an error rather than a
wrap whenever the result leaves the range the crate documents. Years run
from 0 to 9999, which is what a `GeneralizedTime` can write down.

`Instant` and `Duration` are microseconds. An `Instant` has no epoch: it
counts from an origin the caller chooses and only differences of it mean
anything, which is what a timer needs and what a wall clock cannot give.

Two limits are stated rather than left implicit. Leap seconds do not
exist on the POSIX scale, so a `UnixTime` names the same second twice
where UTC inserts one, and the crate does the same. Time zones are not
modeled; every `CivilTime` is UTC.
