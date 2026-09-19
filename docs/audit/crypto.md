# crypto audit findings — one GitHub issue each

Repository: AuDHSOS/AuDHSOS. Audit of `crates/crypto` at main `bc47df5`.
Each `## ` section is one issue, filed 2026-09-18 with the number in its heading. `Title:` is the issue title, `Labels:` the labels, everything after `Body:` is the issue body verbatim.

---

## F01 — issue #23
Title: crypto-dh: shared_secret reports TooWide as OutputTooShort
Labels: bug, part::crypto
Body:
`ModpGroup::shared_secret` maps every `BignumError` from `pow_secret` to `DhError::OutputTooShort` (`crates/crypto/dh/src/modp.rs:142-144`).

`check_public` accepts a peer value longer than `MAX_BYTES` when the overhang is zero bytes (`crates/crypto/dh/src/modp.rs:188-198`), but `pow_secret` derives `used` from the byte length (`crates/crypto/bignum/src/modulus.rs:349-352`) and returns `BignumError::TooWide` for it.

Reproduce: a 513-byte peer value with a leading zero byte passes `check_public`, then `shared_secret` fails with `OutputTooShort` although `out` is wide enough.

Fix: strip leading zero bytes before `pow_secret`, or map `TooWide` to `PublicValueOutOfRange`.

---

## F02 — issue #24
Title: crypto-rng: fill reports MessageTooLong as Exhausted
Labels: bug, part::crypto
Body:
`ChaChaRng::fill` maps the error of `apply_keystream` to `RngError::Exhausted` (`crates/crypto/rng/src/chacha.rs:141-143`). The only error `apply_keystream` returns is `AeadError::MessageTooLong`, for a request of 2^32 or more blocks.

`RngError::Exhausted` documents itself as reported by the scripted generator only (`crates/crypto/rng/src/error.rs:29-32`).

Fix: split an over-long request into runs below the counter range, or add a variant for it.

---

## F03 — issue #25
Title: crypto-rng: one request can exceed RESEED_BYTES under one key
Labels: bug, part::crypto
Body:
`fill` reseeds when `wanted > budget`, then serves the whole request from one key and saturates the budget to zero (`crates/crypto/rng/src/chacha.rs:134-146`). A request of 2 MiB produces 2 MiB from one key, twice `RESEED_BYTES`.

The README states the generator reseeds after a fixed number of bytes (`crates/crypto/rng/README.md:12-17`).

Fix: serve a request in chunks of at most the remaining budget, reseeding between chunks, or document the bound as per-request.

---

## F04 — issue #26
Title: crypto-hash: README claims generated constants, the code holds tables
Labels: bug, part::crypto
Body:
`crates/crypto/hash/README.md:14-17` states the round constants and initial values are generated from their definition in FIPS 180-4 rather than copied from a table.

`crates/crypto/hash/src/sha256.rs:23-101` and `crates/crypto/hash/src/sha512.rs:28-135` hold them as literal arrays.

Fix: either generate the constants in a `const fn` from the primes, or correct the README. A test that derives them from the primes and compares would settle the claim either way.

---

## F05 — issue #27
Title: crypto-hash: the message length wraps silently past 2^61 bytes
Labels: enhancement, part::crypto
Body:
`Sha256::update` and `Sha256::finish` use `wrapping_add` and `wrapping_mul(8)` on the byte counter (`crates/crypto/hash/src/sha256.rs:130-138`). A message of 2^61 bytes or more pads with a wrong bit length and produces a digest of another message with no error.

The README documents the bound at the crate level only (`crates/crypto/hash/README.md:19-21`).

Fix: saturate the counter and state the consequence on `update`, or `debug_assert!` the bound. SHA-512 counts in `u64` bytes and multiplies in `u128` (`crates/crypto/hash/src/sha512.rs`), which is safe.

---

## F06 — issue #28
Title: crypto-bignum: pow_wide writes a partial result before OutputTooShort
Labels: bug, part::crypto
Body:
`write_be` fills `out`, writes the low bytes, and only then reports `OutputTooShort` (`crates/crypto/bignum/src/modulus.rs:367-377`). `pow` and `pow_wide` call it with no length check up front (`crates/crypto/bignum/src/modulus.rs:182-190`), so a too-short `out` holds a truncated value on error.

`pow_secret` checks `out.len() < self.width()` before the ladder (`crates/crypto/bignum/src/modulus.rs:229-231`) and is not affected.

Fix: check `out.len()` before the exponentiation in `pow_wide`, or state in the doc comment that `out` is unspecified on error.

---

## F07 — issue #29
Title: crypto-dh: a refused value stays in out
Labels: bug, part::crypto
Body:
`shared_secret` returns `DegenerateSharedSecret` with the degenerate value still in `out` (`crates/crypto/dh/src/modp.rs:145-147`). `public_value` returns `PublicValueOutOfRange` the same way (`crates/crypto/dh/src/modp.rs:116-121`).

`crypto-aead::open` clears the buffer on refusal so a caller that ignores the result finds nothing usable (`crates/crypto/aead/README.md:16-18`).

Fix: `wipe(out)` before returning either error.

---

## F08 — issue #30
Title: crypto-dh: a non-residue peer value leaks the low bit of the exponent
Labels: enhancement, part::crypto
Body:
`check_public` enforces `1 < v < p-1` only (`crates/crypto/dh/src/modp.rs:92-98`). For the safe prime of group 14 a peer value of order `2q` is a quadratic non-residue, and `v^x` is a residue exactly when `x` is even. The Legendre symbol of the shared secret then tells the peer the low bit of the private exponent.

RFC 8268, section 4, does not require more than the interval check, and the loss is one bit of a 256-bit exponent (`crates/crypto/dh/src/group.rs:49-58`). This is a note, not a defect against the specification.

Options: check `v^q == 1` before use (one extra exponentiation with a public exponent), or record the accepted leak in the README.

---

## F09 — issue #31
Title: crypto-dh: ModpGroup::new accepts any odd modulus
Labels: enhancement, part::crypto
Body:
`ModpGroup::new` takes the prime as bytes and checks only what `Modulus::new` checks: odd, normalized, at most 64 limbs (`crates/crypto/dh/src/modp.rs:46-64`).

The subgroup argument behind `check_public` (`crates/crypto/dh/src/modp.rs:73-79`) holds for a safe prime. A composite or non-safe modulus passes construction and gets the same checks.

Fix: state in the doc comment of `new` that the caller vouches for a safe prime, or restrict construction to the RFC 3526 constants.

---

## F10 — issue #32
Title: crypto-aead: ChaCha20 does not wipe its key on drop
Labels: bug, part::crypto
Body:
`ChaCha20` holds the key as `[u32; 8]` with no `Drop` (`crates/crypto/aead/src/chacha20.rs:23-27`). `ChaCha20Poly1305` holds a `ChaCha20` (`crates/crypto/aead/src/chachapoly.rs:28-32`) and inherits it.

`Poly1305` clears its multiplier, addend, accumulator, and buffer on drop (`crates/crypto/aead/src/poly1305.rs:269-277`), which is the rule the README states for every primitive that sees a key (`crates/crypto/aead/README.md:7-9`).

Fix: a `Drop` on `ChaCha20` that zeros `key` and passes it through `black_box`, as `Poly1305` does.

---

## F11 — issue #33
Title: crypto-aead: AES round keys, the GHASH key, and the key schedule are not wiped
Labels: bug, part::crypto
Body:
Three places hold AES-GCM key material with no wipe:

- `Aes` holds 11 or 15 sliced round keys with no `Drop` (`crates/crypto/aead/src/aes.rs:55-60`).
- `AesGcm::hash_key` and `GHash::key` hold `H = E_K(0)` with no `Drop` (`crates/crypto/aead/src/aesgcm.rs:37-42`, `crates/crypto/aead/src/ghash.rs:25-34`).
- `expand_128` and `expand_256` return the unsliced key schedule by value; the 176 or 240 bytes stay on the stack after `new_128` and `new_256` return (`crates/crypto/aead/src/aes.rs:383-414`, `crates/crypto/aead/src/aes.rs:418-453`).

Fix: `Drop` on `Aes`, `AesGcm`, and `GHash`; `wipe` the expanded schedule in `new_128` and `new_256` before it goes out of scope.

---

## F12 — issue #34
Title: crypto-hash: HMAC and hash states carry key-derived words with no wipe
Labels: bug, part::crypto
Body:
`Hmac` holds two hash states whose chaining words derive from the key and has no `Drop` (`crates/crypto/hash/src/hmac.rs:24-30`). `Sha256` and `Core` have no `Drop` either (`crates/crypto/hash/src/sha256.rs:104-114`, `crates/crypto/hash/src/sha512.rs:139-149`).

Two temporaries hold key material and are not wiped: the digest of a key longer than one block in `Hmac::new` (`crates/crypto/hash/src/hmac.rs:39`), and the 64-byte digest `Sha384::finish` truncates (`crates/crypto/hash/src/sha512.rs:320-326`).

The padded key is wiped (`crates/crypto/hash/src/hmac.rs:61`), so the module invariant covers one of the three copies.

Fix: `Drop` on `Hmac`, `Sha256`, `Core` that zeros `state` and `buffer`; `wipe` the two temporaries.

---

## F13 — issue #36
Title: crypto-hash: Prk is Copy and cannot be wiped
Labels: enhancement, part::crypto
Body:
`Prk<H>` implements `Copy` (`crates/crypto/hash/src/hkdf.rs:29-33`), so it cannot implement `Drop`, and every copy of a pseudorandom key stays where it was copied. `expand` keeps the previous output block in a plain `H::Output` (`crates/crypto/hash/src/hkdf.rs:73-86`).

The TLS key schedule runs entirely through this type, so traffic secrets have no wipe at all.

Fix: drop `Copy`, add `Drop` that wipes `bytes`, and `wipe` `previous` in `expand`. The comment at `crates/crypto/hash/src/hkdf.rs:29-32` gives the reason for `Copy`; a `Clone` call at the key schedule's one hand-over point costs one line.

---

## F14 — issue #37
Title: crypto-bignum: pow_secret leaves the ladder registers on the stack
Labels: bug, part::crypto
Body:
`pow_secret` holds `power`, `ahead`, `product`, `square`, and `result`, five `[u64; 64]` arrays derived from the secret exponent, and returns without wiping them (`crates/crypto/bignum/src/modulus.rs:239-290`). `ct_swap_u64` and the masked subtraction keep the ladder unobservable in time; the values stay in memory.

Fix: `wipe` the five arrays before `write_be` returns. `crypto_ct::wipe` takes `&mut [u8]`; a `wipe_u64` in `crypto-ct` would serve `ct_swap_u64`'s callers.

---

## F15 — issue #38
Title: crypto-ec: ed25519 sign and expand leave the secret scalar, prefix, and nonce on the stack
Labels: bug, part::crypto
Body:
`expand` keeps the 64-byte digest of the secret, the clamped scalar, and the prefix in plain arrays (`crates/crypto/ec/src/ed25519.rs:397-413`). `sign` keeps `scalar`, `prefix`, and the nonce `r` (`crates/crypto/ec/src/ed25519.rs:367-392`). `Scalar` has no `Drop` (`crates/crypto/ec/src/scalar.rs:36-37`).

`Point::mul_secret` and `Scalar::mul_secret` hide these values in time; nothing clears them afterwards.

Fix: `wipe` the arrays at the end of `expand` and `sign`; give `Scalar` a `clear` for the two scalars.

---

## F16 — issue #39
Title: crypto-ec: the x25519 ladder leaves the clamped scalar and its registers on the stack
Labels: bug, part::crypto
Body:
`ladder` copies the scalar into `clamped` and returns without wiping it (`crates/crypto/ec/src/x25519.rs:65-101`). `x2`, `z2`, `x3`, `z3` are the secret-derived working points and are not cleared either.

Fix: `wipe(&mut clamped)` and zero the four `Fe` registers before the return.

---

## F17 — issue #40
Title: crypto-rng: rekey leaves the next key in a keystream block on the stack
Labels: bug, part::crypto
Body:
`rekey` computes a full 64-byte keystream block, copies its first 32 bytes into the key, and drops the block unwiped (`crates/crypto/rng/src/chacha.rs:120-126`). The block holds the current key of the generator until the stack slot is reused.

`reseed` wipes `fresh` (`crates/crypto/rng/src/chacha.rs:92`), so the pattern exists two functions up.

Fix: `wipe(&mut block)` after the copy.

---

## F18 — issue #41
Title: crypto-ec: Scalar and Fe derive Debug
Labels: bug, part::crypto
Body:
`Scalar` derives `Debug` (`crates/crypto/ec/src/scalar.rs:36`) and holds the Ed25519 secret scalar and the signing nonce. `Fe` derives `Debug` (`crates/crypto/ec/src/fe25519.rs:26`) and holds the X25519 ladder registers and the shared secret.

`Secret<N>` states the rule: the bytes never reach a formatter (`crates/crypto/ct/src/secret.rs:90-95`). `ChaCha20`, `Aes`, `Poly1305`, and `Hmac` follow it by deriving nothing.

Fix: replace the derives with a `Debug` that prints the type name, as `Element` does (`crates/crypto/ec/src/montgomery.rs:78-82`).

---

## F19 — issue #42
Title: crypto-ec: Scalar derives PartialEq
Labels: enhancement, part::crypto
Body:
`Scalar` derives `PartialEq` and `Eq` (`crates/crypto/ec/src/scalar.rs:36`), which is a byte-wise comparison that stops at the first differing limb. `is_zero` uses it (`crates/crypto/ec/src/scalar.rs:160-162`).

`verify` compares only public scalars, so no call today leaks. `Secret<N>` withholds `==` so that a comparison of secrets has to go through `ct_eq` (`crates/crypto/ct/src/secret.rs:19-22`).

Fix: drop the derive and offer `ct_eq` returning `Choice`, or keep the derive and state on the type that `==` is for public scalars.

---

## F20 — issue #43
Title: crypto-ct: Choice has no optimizer barrier
Labels: enhancement, part::crypto
Body:
`Choice` is a plain `u8` newtype (`crates/crypto/ct/src/choice.rs:21`). LLVM can see through `mask_u8`, `ct_select_*`, and the masked scans that select a byte by comparing an index — `bit_of` in `crates/crypto/bignum/src/modulus.rs:400-406` and `crates/crypto/ec/src/x25519.rs:115-123` — and rewrite them as a branch or an indexed load. The `subtle` crate passes every `Choice` through `core::hint::black_box` on construction for this reason.

`Secret::clear` already uses `black_box` as the barrier for wiping (`crates/crypto/ct/src/secret.rs:66-71`).

Fix: `black_box` the byte in `Choice::from_lsb`, `is_zero_u8`, `is_zero_u64`, and `From<bool>`; add a test that inspects the generated assembly of `ct_select_u64` for a conditional branch, or accept the risk in the README next to the wipe caveat.

---

## F21 — issue #44
Title: crypto-ct: wipe passes a reference to the reference to black_box
Labels: enhancement, part::crypto
Body:
`wipe(bytes: &mut [u8])` calls `black_box(&bytes)` (`crates/crypto/ct/src/secret.rs:79-82`). The argument has type `&&mut [u8]`, one indirection further from the buffer than `Secret::clear`'s `black_box(&self.bytes)` (`crates/crypto/ct/src/secret.rs:68-71`).

Fix: `black_box(&*bytes)` or `black_box(bytes)` after the fill.

---

## F22 — issue #45
Title: crypto-ct: Choice derives PartialEq
Labels: enhancement, part::crypto
Body:
`Choice` derives `PartialEq` and `Eq` (`crates/crypto/ct/src/choice.rs:20`). `choice == Choice::YES` is `is_true` without the name that documents the exit from constant time (`crates/crypto/ct/src/choice.rs:88-97`).

Fix: drop the derive. Tests that compare choices can compare `value()`.

---

## F23 — issue #46
Title: crypto-aead: GHASH multiplies bit by bit
Labels: enhancement, part::crypto
Body:
`multiply` runs 128 iterations of `u128` shift, mask, and xor per block (`crates/crypto/aead/src/ghash.rs:103-113`): O(128) word operations for 16 bytes of input, so the tag costs more than the cipher.

A table-free alternative with the same access pattern: split into 64-bit halves, carry-less multiply by masked shift-and-add over `u64` (Karatsuba, three products), then one fixed reduction. That is O(64) per product and a quarter of the work.

The module documents the choice (`crates/crypto/aead/src/ghash.rs:7-11`); this is a performance note, not a defect.

---

## F24 — issue #47
Title: crypto-aead: AES-GCM computes lanes it does not use
Labels: enhancement, part::crypto
Body:
Two places pay for four AES blocks to use fewer:

- `keystream` encrypts four counter blocks for a remainder of one to three blocks (`crates/crypto/aead/src/aesgcm.rs:87-92`, `crates/crypto/aead/src/aesgcm.rs:97-111`).
- `encrypt_block` replicates one block into four lanes (`crates/crypto/aead/src/aes.rs:95-100`); `AesGcm::new` calls it for `H` and `tag` calls it for the mask (`crates/crypto/aead/src/aesgcm.rs:46-50`, `crates/crypto/aead/src/aesgcm.rs:140-141`).

A TLS record of 100 bytes pays eight block encryptions for two blocks of keystream plus the mask.

Fix: put the tag-mask counter block into a lane of the first group of a message, and `H` into a spare lane of the first `keystream` call under a key or accept the cost once per key.

---

## F25 — issue #48
Title: crypto-hash: HKDF expand rebuilds the HMAC key schedule per block
Labels: enhancement, part::crypto
Body:
`expand` calls `Hmac::new(prk)` in every iteration (`crates/crypto/hash/src/hkdf.rs:75-82`). Each call absorbs two padded blocks, so a 255-block expansion pays 510 extra compressions.

`Hmac` is `Clone` (`crates/crypto/hash/src/hmac.rs:24`).

Fix: build the keyed state once before the loop and clone it per block.

---

## F26 — issue #49
Title: crypto-hash: finish pads one byte per absorb call
Labels: enhancement, part::crypto
Body:
`Sha256::finish` and `Core::finish` absorb the padding one zero byte at a time until the buffer reaches the length field (`crates/crypto/hash/src/sha256.rs:139-142`, `crates/crypto/hash/src/sha512.rs:169-175`): up to 63 or 127 calls to `absorb`, each with the buffer bookkeeping.

Fix: compute the count once and absorb one slice of `ZERO_BLOCK`.

---

## F27 — issue #50
Title: crypto-ec: signature verification runs two full scalar multiplications
Labels: enhancement, part::crypto
Body:
ECDSA verify computes `u1·G` and `u2·Q` as two complete double-and-add chains and adds the results (`crates/crypto/ec/src/p256/mod.rs:107-109`, `crates/crypto/ec/src/p384/mod.rs:111-113`). Shamir's trick shares the doublings: one chain of `n` doublings and at most `2n` additions, against `2n` doublings today.

Ed25519 verify multiplies the base point by `s` with no fixed-base precomputation (`crates/crypto/ec/src/ed25519.rs:335`).

Both are public-scalar paths and may branch, so the change is arithmetic only. Performance note.

---

## F28 — issue #51
Title: crypto-bignum: Modulus is a 1 KiB Copy type carried by value
Labels: enhancement, part::crypto
Body:
`Modulus` holds two `[u64; 64]` arrays and derives `Copy` (`crates/crypto/bignum/src/modulus.rs:24-36`). `ModpGroup` embeds it plus a 512-byte `upper` (`crates/crypto/dh/src/modp.rs:19-31`); `PublicKey` embeds it (`crates/crypto/rsa/src/key.rs:17-26`). `pow_secret` adds seven `[u64; 64]` arrays (`crates/crypto/bignum/src/modulus.rs:224-290`).

One shared-secret call is about 5 KiB of stack, before the caller's own frames. A 2048-bit group uses half of each array.

Options: drop `Copy` so a move is explicit, or make the width a const generic so a 2048-bit group is 2048 bits wide.
