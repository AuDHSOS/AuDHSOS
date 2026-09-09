# SPDX-License-Identifier: AGPL-3.0-only
# Copyright (C) 2026 Manuel Baesler and contributors

# Sequential release measurements, seven samples of ten isolated runs each.
# Compilation is outside execute_total; no Node or external benchmark package.
set -eu
cd "$(dirname "$0")/.."
# Build with the pinned toolchain once. Measure direct launches, matching the
# before/after protocol; cargo/xtask launch context measurably changes timings
# on the development host even though process startup is not in execute_total.
sh tools/xtask.sh jrs --help >/dev/null
for name in sum arithmetic calls; do
    sample=1
    while [ "$sample" -le 7 ]; do
        target/release/jrs --stats --bench 10 "tools/benchmarks/jrs/$name.js"
        sample=$((sample + 1))
    done
done
