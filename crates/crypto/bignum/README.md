# crypto-bignum

The wide arithmetic the asymmetric algorithms of this track share: limbs
of sixty-four bits held least significant first, Montgomery
multiplication over a modulus passed as an argument, and modular
exponentiation over a modulus that is not known until a certificate is
read.

Two callers, and they need different things from the same code.
`crypto-ec` knows its four moduli when it is compiled — the field and the
group order of P-256 and of P-384 — and carries them as associated
constants; it takes the three free functions of `limbs` and nothing else.
`crypto-rsa` learns its modulus from a key on the wire, so it takes
`Modulus`, which derives at run time what `crypto-ec` writes down: the
negative inverse of the low limb modulo `2^64`, and `2^(128*N)` modulo
the modulus.

One width, chosen at run time. A `Modulus` holds sixty-four limbs, four
thousand and ninety-six bits, together with the count of limbs actually
in use, and every loop runs over that count. Limbs at or above it are
zero in every value the crate holds, which is the invariant everything
else rests on. A two-thousand-and-forty-eight-bit key therefore costs a
two-thousand-and-forty-eight-bit multiplication and a
four-thousand-and-ninety-six-bit stack frame.

## Nothing here is constant time, and that is the design

The crates around this one state where their branches may not depend on a
value. This one states the opposite, and owes the same explanation.

Every value that reaches this crate is public. A modulus arrives in a
certificate, an exponent is three or sixty-five thousand five hundred and
thirty-seven, and a signature is on the wire. So the final conditional
subtraction of a Montgomery product branches on the value it just
computed, and the exponentiation is left-to-right square-and-multiply,
which branches on every bit of the exponent. Both are written that way on
purpose, because the alternative buys nothing and hides the arithmetic.

What holds this up is the boundary rather than the code: no secret ever
enters this crate. Nothing in the track signs outside the `test-signing`
features, key exchange is `crypto-ec`'s constant-time ladder and does not
come here, and `crypto-rsa` verifies and does not decrypt. Should that
ever change, this crate is the wrong tool and the change is the place to
say so.
