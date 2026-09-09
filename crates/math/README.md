# Binary64 math

Safe, allocation-free `no_std` numerical kernels without external dependencies,
platform libm, FFI or architecture-specific instructions.

`pow` implements the binary64 special cases of local ECMA-262 §6.1.6.1.3,
including NaN-to-zero, signed zeros, infinite exponents and odd integer signs.
Finite positive magnitudes use normalized mantissa/exponent decomposition,
an atanh series for logarithms and an exponential series after ln(2) reduction.
Two-component arithmetic preserves low bits through range reduction, especially
for bases near one with large exponents. Series have fixed iteration counts;
there is no input-controlled convergence loop, allocation or unbounded work.
Subnormal scaling is explicit. The result is implementation-approximated,
not a claim of correctly rounded transcendental arithmetic on every input.
Small integer exponents (absolute value at most 16, magnitudes between 2^-32
and 2^32) use bounded two-component repeated squaring; exponents 1, 2 and -1
have direct paths. Tests include exact representable powers of two, special-value
matrices and deterministic host-pow comparisons. `fuzz/math_pow` additionally
compares arbitrary binary64 inputs, normalized mantissas and near-one bases
with large exponents, using the host library only as a test oracle. A four-ULP
comparison tolerance is a test criterion, not a proven global error bound.

The design prioritizes bounded portable execution and numerical accuracy.
Table/polynomial acceleration and a comprehensive worst-case ULP proof remain
future work; competitive speed is not established by these kernels alone.

`RadixInteger` accumulates base-2 through base-36 digits exactly and rounds only
once to binary64 (nearest, ties to even). Its fixed 32-limb buffer stores up to
1024 bits; larger nonnegative integers necessarily round to infinity. Push is
bounded by 32 multiply/add operations, and small values use the u64 conversion
fast path. It has no allocation, input-length-dependent stack or external
dependency. The caller supplies lexical validation, signs and a total scan/work
budget. Tests exercise halfway/sticky/carry bits, the finite/infinity boundary,
all radices versus u128, and long decimal inputs against Rust's decimal parser.
The `math_pow` fuzz target covers the integer converter as well as power.
