# SPDX-License-Identifier: AGPL-3.0-only
# Copyright (C) 2026 Manuel Baesler and contributors
#
# A stand-in for the suite's `fuzz_common.tcl`.

proc fuzz_do {args} {}
proc Expr {args} { return "1" }
proc Select {args} { return "SELECT 1" }
