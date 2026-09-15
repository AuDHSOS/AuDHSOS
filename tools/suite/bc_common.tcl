# SPDX-License-Identifier: AGPL-3.0-only
# Copyright (C) 2026 Manuel Baesler and contributors
#
# A stand-in for the suite's `bc_common.tcl`, which drives a second
# build of the C library beside the first.

proc do_bc_test {args} {}
proc bc_find_binaries {args} { return {} }
