# jrs performance measurements

The performance requirement is **not yet satisfied**. These measurements show a
specific interpreter improvement, not competitiveness with production JS engines.
No benchmark result changes conformance expectations or resource limits.

## Number/Number stack fast path (2026-09-09)

The baseline and candidate contain the same language features and Function source
preservation. The candidate replaces pure Number/Number binary arithmetic and
comparisons in place on the operand stack. It avoids popping two large Value
enums, generic conversion dispatch and re-pushing the result. Other operand types
and operators keep the existing path. It is not constant folding, JIT compilation,
fast-math, or a relaxation of IEEE special-value behavior.

Development host: macOS/aarch64, pinned nightly-2026-08-25 toolchain, release
profile. Seven sequential samples per scenario, ten isolated executions per
sample, compiled once per sample. `execute_total` excludes compilation, process
startup and CLI output. Samples were measured before and after rebuilding, not
interleaved; no CPU-frequency pinning or confidence interval is claimed.

| Scenario | Baseline median (ms) | Candidate median (ms) | Lower execution time |
|---|---:|---:|---:|
| Sum, 100,000 iterations | 109.885000 | 96.227667 | 12.4% |
| Mixed arithmetic, 100,000 iterations | 132.147667 | 111.924084 | 15.3% |
| Function calls, 20,000 iterations | 55.762833 | 50.419209 | 9.6% |

Sorted samples in milliseconds:

```text
sum baseline: 109.165208 109.494750 109.739584 109.885000 110.340542 111.766167 113.836250
sum candidate: 95.563083 96.019666 96.202208 96.227667 96.301667 98.749750 100.784041
arithmetic baseline: 130.977041 131.817125 132.016459 132.147667 133.132459 133.367250 134.990042
arithmetic candidate: 110.167541 110.828917 110.990375 111.924084 112.448417 112.889500 113.883334
calls baseline: 54.239167 54.956584 55.724542 55.762833 56.171417 56.274541 56.532125
calls candidate: 49.723875 49.799375 50.402708 50.419209 50.674542 50.753000 51.105041
```

The exact scenario source is in `tools/benchmarks/jrs/`. To collect new samples:

```sh
sh tools/jrs-bench.sh
```

Expected final results: sum `4999950000`, arithmetic `3`, calls `20000`. The
benchmark source adds only comments to the original inline measurement strings.
The script builds once with the project's toolchain wrapper, then launches the
binary directly, matching the before/after measurements. It uses no external
benchmark package.
Do not compare concurrent full-check/fuzzing runs to these quiet-host samples.

Launch-context check: five direct inline sum samples were 98.694, 96.368,
96.176208, 98.180375 and 97.022834 ms; five direct file samples were 100.524250,
95.487709, 95.277958, 96.991292 and 98.211500 ms. Five launches via the
cargo/xtask wrapper measured 122.589500, 122.548458, 126.409708, 119.420542 and
109.723959 ms. This demonstrates a launch-context dependency on this host, not
its cause. The benchmark script therefore does not launch through Cargo for
each sample. The initial wrapper-per-sample run is retained in
`target/jrs-bench-numeric-fast.txt` and must not be compared with the direct-launch
baseline as if the conditions were identical.

The corrected script completed with all expected results. Its direct-launch
repeat recorded the following raw samples (`target/jrs-bench-numeric-direct.txt`):

```text
sum: 113.267208 96.210500 95.872000 95.568459 98.043709 97.986541 100.015875
arithmetic: 112.130416 112.672584 112.029292 112.236208 112.237834 112.111125 113.305000
calls: 51.745333 52.192250 51.984375 51.893875 51.653541 51.536292 51.438000
```

Medians are 97.986541, 112.236208 and 51.745333 ms respectively, consistent with
the first candidate measurements but not numerically identical. Compared with
the earlier baseline medians, these are 10.8%, 15.1% and 7.2% lower. The first
sum sample is retained; it is not discarded as a warm-up. This repeat strengthens
the local result, but does not replace interleaved controlled A/B measurements.

## Correctness contract

The dispatch still charges one fuel unit per instruction. This fast path cannot
allocate, call a getter/conversion callback, run GC, or grow the operand stack.
The two input values have already passed stack checks. It replaces one input
slot and removes the other, leaving preceding stack values unchanged. A miss
does not modify operands, so the generic path retains exception and conversion
order. Tests compare every supported operator with generic evaluation over
special values and 10,000 deterministic binary64 pairs, allowing arbitrary NaN
payloads but requiring signed-zero bits and all non-NaN results to agree exactly.
Boundary tests verify fuel exhaustion, stack capacity, mixed-type fallbacks,
callback order and async suspension.

## Number predicate follow-up (2026-09-09)

The numeric-parser runs recorded medians of 100.081 / 118.924 / 51.428 ms and
103.600 / 114.738 / 52.275 ms (sum/arithmetic/calls). These were higher than the
previous splice repeat (96.933 / 110.926 / 50.522 ms); no unchanged-performance
claim was made. An exact immediately-pre-parser binary was not preserved, so
the cause of that difference remains unproven.

Before adding Number predicates, the current release executable was copied to
`target/jrs-before-number-predicates`. After building, ten paired samples per
workload alternated before/after and after/before launch order. Each sample ran
ten isolated executions, with compilation excluded. No project check or fuzzing
ran concurrently. Every sample and the executable SHA-256 hashes are retained
in `target/jrs-predicates-ab.txt`; the diagnostic driver is
`target/jrs-predicates-ab.cjs`. No initial sample was discarded.

| Scenario | Before median (ms) | After median (ms) | Median paired after/before |
|---|---:|---:|---:|
| Sum | 103.832 | 100.419 | 0.9751 |
| Arithmetic | 118.696 | 113.009 | 0.9586 |
| Calls | 52.448 | 51.384 | 0.9811 |

This rules out a slowdown from the predicate addition in this measured sample,
not a general regression or the earlier parser change. These workloads do not
invoke Number predicates; code-layout effects and host drift remain plausible.
This is not evidence that predicates accelerate arithmetic or that jrs is
competitive with production engines. The earlier performance gap remains open.

Further work remains: broader workloads, persistent-realm performance, allocation
and memory accounting, real application benchmarks, reproducible cross-engine
comparisons and whole-runtime profiling. These microbenchmarks do not prove a
fast JavaScript runtime by themselves.

## Sort widening and toSorted paired run (2026-09-09)

The preceding iterator widening had sequential medians around 100–101 / 116–117 /
51–54 ms. The exact pre-sort executable (already containing the iterator changes)
was copied to `target/jrs-before-sort-wide`. Ten before/after pairs alternated
launch order for each unchanged workload, ten isolated runs per sample, without
concurrent project tests. Raw samples and SHA-256 hashes are in
`target/jrs-sort-ab.txt`; driver: `target/jrs-sort-ab.cjs`. No sample was removed.

| Scenario | Before median (ms) | After median (ms) | Median paired after/before |
|---|---:|---:|---:|
| Sum | 99.996 | 98.958 | 0.9933 |
| Arithmetic | 113.573 | 113.867 | 1.0014 |
| Calls | 50.792 | 51.438 | 1.0115 |

This sample shows no large additional slowdown from sort widening/toSorted;
it cannot identify the cause of the earlier iterator-era change, because both
binaries include that change. The benchmark programs do not call sort. Host drift,
code layout and GC/intrinsic overhead are not isolated here, and this is not a
statistical equivalence claim or proof of competitive runtime performance.
