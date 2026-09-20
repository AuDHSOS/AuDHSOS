# crypto-ct

The constant-time vocabulary the cryptography crates share: `Choice`, a
branchless boolean that carries the result of a comparison without ever
becoming a branch; comparison and selection over that boolean; and
`Secret<N>`, the container key material lives in.

The rule the whole track follows is stated here and repeated in each
crate that handles secrets: no branch and no index on a secret value.
Lengths, protocol constants, and certificate contents are public. Keys,
shared secrets, traffic secrets, and plaintext are not.

`Choice` passes each byte it derives from a value through
`core::hint::black_box`, so the optimizer cannot prove the byte is `0` or
`1` and turn a mask into a branch or an indexed load; the two named
constants and the operators need none. Neither `Choice` nor
`Secret<N>` carries `PartialEq`, so a comparison names itself:
`Choice::is_true` for the deliberate exit from constant time,
`Secret::ct_eq` for the comparison that folds every byte.

`wipe`, `wipe_u32`, and `wipe_u64` are the same erase for a buffer that
cannot be a `Secret`: the padded key inside HMAC, the key words of a
stream cipher, the limb arrays of a modular exponentiation.

One limit is honest rather than hidden. Erasing memory reliably needs a
volatile write, and a crate that forbids `unsafe` has none. `Secret<N>`
overwrites its bytes on drop and passes the buffer through
`core::hint::black_box`, which stops the optimizer that exists today from
removing the writes. It is not a guarantee. Key material therefore lives
as briefly as the protocol allows and never leaves the connection state.
