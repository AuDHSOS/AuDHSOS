# crypto-bignum

The wide arithmetic the asymmetric algorithms of this track share: limbs
of sixty-four bits held least significant first, Montgomery
multiplication over a modulus passed as an argument, and modular
exponentiation over a modulus that is not known until a certificate is
read.

Three callers, and they need different things from the same code.
`crypto-ec` knows its four moduli when it is compiled — the field and the
group order of P-256 and of P-384 — and carries them as associated
constants; it takes the free functions of `limbs` and nothing else.
`crypto-rsa` learns its modulus from a key on the wire, so it takes
`Modulus`, which derives at run time what `crypto-ec` writes down: the
negative inverse of the low limb modulo `2^64`, and `2^(128*N)` modulo
the modulus. `crypto-dh` takes the same `Modulus` for a prime it knows in
advance, and is the one caller whose exponent is secret.

One width, chosen at run time. A `Modulus` holds sixty-four limbs, four
thousand and ninety-six bits, together with the count of limbs actually
in use, and every loop runs over that count. Limbs at or above it are
zero in every value the crate holds, which is the invariant everything
else rests on. A two-thousand-and-forty-eight-bit key therefore costs a
two-thousand-and-forty-eight-bit multiplication and a
four-thousand-and-ninety-six-bit stack frame.

## Two arithmetics, and the boundary between them

The crates around this one state where their branches may not depend on a
value. This one has to state it twice, because it holds both kinds.

The public arithmetic is the older half and still the larger one. A
modulus arrives in a certificate, an RSA public exponent is three or
sixty-five thousand five hundred and thirty-seven, and a signature is on
the wire, so `montgomery` ends in a conditional subtraction that branches
on the value it just computed, and `pow` and `pow_wide` are left-to-right
square-and-multiply, which branches on every bit of the exponent. Both
are written that way on purpose: for a public exponent the alternative
buys nothing and hides the arithmetic.

The secret arithmetic is `montgomery_secret` and `pow_secret`, and it
exists because finite-field Diffie-Hellman needs it. There the exponent
is the private value of a key exchange, so `pow_secret` is a Montgomery
ladder: eight rounds per byte of the exponent buffer whatever the bytes
are, one squaring and one multiplication in each of them, and a masked
exchange of the two working values rather than a branch on the bit. The
products it uses subtract the modulus always and mask whether the
subtraction counts, so that the intermediate values are as unobservable
as the exponent. What stays public is what a length is: the width of the
modulus and the length of the exponent buffer, both of which set loop
counts and neither of which is a secret.

The boundary is the exponent, and it is the caller who knows which side
of it a call is on. Only `pow_secret` protects one, and it is the only
name here that claims to; a caller reaching for `pow` or `pow_wide` with
a secret has picked the wrong function, and no check in this crate will
catch it.
