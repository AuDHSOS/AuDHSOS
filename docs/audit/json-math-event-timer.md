# audhsos-json, audhsos-math, audhsos-event-target, audhsos-timer-queue audit findings

Repository: AuDHSOS/AuDHSOS. Audit of audhsos-json, audhsos-math, audhsos-event-target, audhsos-timer-queue at main bc47df5.
Each `## ` section is one issue, filed 2026-09-19 with the number in its heading. `Title:` is the issue title, `Labels:` the labels, everything after `Body:` is the issue body verbatim.

---

## F01 — issue #399
Title: audhsos-math: two-constant Cody-Waite reduction loses up to 33 bits of sin below 2^18
Labels: bug
Body:
`reduce_cody_waite` at `crates/math/src/trig.rs:66-71` subtracts `quotient * PIO2_FIRST` and `quotient * PIO2_FIRST_TAIL` from the argument. `PIO2_FIRST` at `crates/math/src/trig.rs:26` carries 31 significant bits and `PIO2_FIRST_TAIL` at `crates/math/src/trig.rs:27` carries 53, so the pair represents pi/2 with an absolute error of 3.5e-27 (2^-87.9). The quotient reaches 166,886 at `crates/math/src/trig.rs:28`, so the reduced argument carries an absolute error of up to 4.1e-22 from the constants alone, plus the rounding of `quotient_float * PIO2_FIRST_TAIL` at the same magnitude.

For the argument `0x41065a1dd290660f` (183107.7278144811, quotient 116,570, true reduced argument -4.5208812e-16) `sin` returns 4.52087741970985e-16 while the correctly rounded value is 4.520881201416264e-16: a relative error of 8.4e-7, 3,835,105,925 ULP. Every argument below 2^18 whose distance to a multiple of pi/2 is below 2^-20 loses more than 20 bits. The test at `crates/math/src/tests/mod.rs:186-220` compares with an absolute tolerance of 2e-15, so it accepts this result.

Fix: reduce with three constants as fdlibm's `__ieee754_rem_pio2` does (33-bit `pio2_1`, 33-bit `pio2_2`, 53-bit `pio2_3` with their tails) and return the reduced argument as a two-component (hi, lo) value that the kernels consume; lowering `CODY_WAITE_LIMIT` so that Payne-Hanek covers more arguments does not help while F02 stands.

---

## F02 — issue #400
Title: audhsos-math: Payne-Hanek remainder extraction rounds the fraction in an f64 accumulator
Labels: bug
Body:
`reduce_payne_hanek` at `crates/math/src/trig.rs:120-129` builds the fractional part of `value * 2/pi` by adding 64 product bits with weights 0.5 down to 2^-64 into one `f64` and subtracts 1.0 when `round_up` is set. The accumulator holds 53 bits, so a fraction above 0.5 is rounded at 2^-53 before the subtraction, and a fraction below 2^-11 keeps fewer than 53 of the 64 bits. The comment at `crates/math/src/trig.rs:9-11` claims more than 500 guard bits; the extraction at `crates/math/src/trig.rs:122-127` reads 64 bits and the accumulator keeps 53 of them.

For `0x436ce638ef85d9f8` (6.507545247185709e16) `sin` returns 4.222466307462429e-6 while the correctly rounded value is 4.222466307313476e-6 (175,852 ULP); for `0x4ac8d999b501d972` (1.859497885449159e52) the result is off by 47,739 ULP. Over 984,028 pseudo-random finite arguments at or above 2^18, 29,846 (3.0%) differ from the host `sin` by more than 4 ULP. For the arguments closest to a multiple of pi/2 (fraction 1 - 2^-60) the accumulator rounds to 1.0 and `reduced` becomes 0.0, so `sin` returns the sign of the argument times 0.0 instead of a value near 2^-60.

Fix: read the bits below `half_bit` into a 128-bit integer, negate it in two's complement when `round_up` is set, normalize by its leading zeros while pulling further bits from `product`, and convert the top 53 and the next 53 bits into a (hi, lo) pair that is multiplied by pi/2 in two-component arithmetic; widening the accumulator to a second `f64` without the integer negation still loses the bits above 2^-53 on the round-up side.

---

## F03 — issue #401
Title: audhsos-json: the parser accepts 49 nested containers while the documented hard cap is 48
Labels: bug
Body:
`value` at `crates/json/src/lib.rs:137` refuses a value whose `depth` exceeds `limits.depth.min(48)`, and a container passes `depth + 1` to its children at `crates/json/src/lib.rs:163` and `crates/json/src/lib.rs:185`. The outermost container is at depth 0, so a container at depth 48 is accepted and its children are refused. `crates/json/README.md:12` states "bounded nesting (hard cap 48)" and `crates/json/src/lib.rs:25` states "Maximum container nesting (also hard-capped at 48)".

`parse` of 49 `[` followed by 49 `]` with `Limits::default()` returns `Ok` with 49 nodes; 50 nested arrays return `Err(Limit)`. 48 nested arrays around a scalar (49 levels of nesting) also parse. The recursion of `value` reaches 49 frames, one more than the documented cap.

Fix: check `depth >= self.limits.depth.min(48)` in the two container arms before the `bump` at `crates/json/src/lib.rs:158` and `crates/json/src/lib.rs:174`, so that the limit counts containers; changing the two documentation lines to describe the off-by-one keeps the extra recursion frame.

---

## F04 — issue #402
Title: audhsos-math: sin tests use an absolute tolerance that accepts any result near zero
Labels: enhancement
Body:
`sine_reduction_paths_agree_with_host_math` at `crates/math/src/tests/mod.rs:200-201` and `crates/math/src/tests/mod.rs:215-218` asserts `(actual - expected).abs() <= 2.0e-15`. Every `sin` result below 2e-15 in magnitude passes regardless of its value, and a result of magnitude 1e-5 passes with 5e10 ULP of error.

The defects of F01 (3.8e9 ULP at 183107.7278144811) and F02 (175,852 ULP at 6.507545247185709e16) pass this test. `crates/math/README.md:15` cites this test as the comparison against the host oracle.

Fix: compare `actual.to_bits().abs_diff(expected.to_bits())` against a ULP bound, as `deterministic_binary64_differential_against_host_math` does at `crates/math/src/tests/mod.rs:79-82` for `pow`, and add the two arguments named in F01 and F02 to the fixed list at `crates/math/src/tests/mod.rs:187-197`.

---

## F05 — issue #403
Title: audhsos-json: the opening quote of every object key is charged twice against the work counter
Labels: enhancement
Body:
The object arm at `crates/json/src/lib.rs:179-182` calls `need(34)`, which consumes the quote through `bump` and charges one unit at `crates/json/src/lib.rs:111`, then steps `at` back by one and calls `string`, whose `need(34)` at `crates/json/src/lib.rs:256` consumes and charges the same quote again.

`parse` of `{"a":1}` (7 code units) charges 8 units; `{"a":1,"b":2}` (13 code units) charges 15. With `work` set to the input length, `{"a":1}` returns `Err(Limit)` while `["a",1]` of the same length returns `Ok`. `crates/json/README.md:12` states parsing is O(n) in input units; the charge exceeds the input length by the number of keys.

Fix: replace the three lines with `if self.peek() != Some(34) { return Err(Error::Syntax(self.at)); }` and let `string` consume the quote; keeping the double charge and documenting it leaves the work counter above the input length.

---

## F06 — issue #404
Title: audhsos-math: the expect reason on positive_pow states a range the reduction exponent leaves
Labels: enhancement
Body:
The `#[expect]` at `crates/math/src/lib.rs:168-172` justifies the `as i32` cast at `crates/math/src/lib.rs:184` with "range-checked exponential reduction is in [-1075,1024]". The check at `crates/math/src/lib.rs:179` admits `approximate` down to -746, and `units` at `crates/math/src/lib.rs:183` is `-746 * log2(e) = -1076.25`, so `n` reaches -1076.

`pow(0.5, 1076.0)` computes `approximate = -745.83`, `units = -1076.0` and `n = -1076`. The cast stays lossless, so the lint suppression holds; the stated range does not.

Fix: change the reason to "[-1076,1024]"; tightening the threshold at `crates/math/src/lib.rs:179` to -745.2 would also exclude -1076 but changes the point where results flush to zero.
