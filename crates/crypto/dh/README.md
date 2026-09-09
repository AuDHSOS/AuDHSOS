# crypto-dh

Finite-field Diffie-Hellman over a MODP group: the key exchange that
RFC 9142, table 12, makes the single MUST for SSH, under the name
`diffie-hellman-group14-sha256`.

The method is two exponentiations modulo a fixed 2048-bit prime. One
side raises the generator to its private exponent and sends the result,
the other side raises what it received to its own private exponent, and
both arrive at the same value. This crate is those two operations, the
group they run in, and the range check that has to happen before a
received value is used. It is not the SSH method: the messages, the
exchange hash and the negotiation live above it.

## What the group is

`ModpGroup` is a prime and a generator, and the constants Montgomery
arithmetic derives from the prime. The prime of group 14 is the one
RFC 3526, section 3, prints, and its generator is two. The type is
written for a group that arrives as bytes rather than for that one group
alone, so the other MODP groups of the same document that the arithmetic
is wide enough for — group 15 at 3072 bits and group 16 at 4096 — are a
constant away.

## What is secret and what is not

The private exponent is the secret, and it is the only one. The prime,
the generator, and both public values are on the wire. So the
exponentiation is `crypto-bignum`'s `pow_secret`, a Montgomery ladder
that takes the same number of rounds and the same memory accesses
whatever the exponent is, and the shared secret is compared against the
values it must not take with `crypto-ct`'s constant-time equality. The
range check on a peer's public value is deliberately ordinary
comparison: that value came from the network and is public by
construction.

## The check on a received value

RFC 4253, section 8, requires that `e` and `f` lie in a range, and gets
the range wrong. RFC 8268, section 4, corrects it to the open interval:
`1 < e < p-1` and `1 < f < p-1` MUST hold, and the exchange fails
otherwise. That is what `check_public` enforces, and the reason the
document gives is the one that matters — a value of one or of `p-1`
forces the exchange into a subgroup with one or two elements, and the
shared secret is then known to anybody watching.

## The length of the private exponent

RFC 4253 draws the exponent from `1 < x < q`, where `q` is the order of
the subgroup, which for the safe prime of group 14 is 2047 bits.
Implementations do not use the whole range and do not need to: the best
generic attack on a short exponent is a square root of its range, so an
exponent of 256 bits costs 2^128 to recover, well above the 112 bits of
security RFC 9142, table 4, credits the 2048-bit group with. The
exponent is therefore not the weak part of this exchange at that length,
and `GROUP14_SECRET_BYTES` says so. It is a recommendation and not a
restriction: the operations take an exponent of any length, and the
length they are given is the number of rounds they run.
