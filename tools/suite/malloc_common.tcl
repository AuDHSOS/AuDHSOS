# SPDX-License-Identifier: AGPL-3.0-only
# Copyright (C) 2026 Manuel Baesler and contributors
#
# A stand-in for the suite's `malloc_common.tcl`: every command in it
# asks the C library to fail an allocation, which this harness cannot.

# `MEMDEBUG` says whether the library fails an allocation where a case
# asks it to, which the suite's own file sets from `builtin_test` and
# which is nought here. A file reads it to skip the cases it writes for
# such a failure.
set ::MEMDEBUG 0

proc do_malloc_test {args} {}
proc do_faultsim_test {args} {}
proc do_one_faultsim_test {args} {}
proc faultsim_integrity_check {args} {}
proc faultsim_test_result {args} {}
proc faultsim_test_control {args} {}
proc do_write_test {args} {}
proc sqlite3_memdebug_vfs_oom_test {args} { return 0 }
set ::FAULTSIM(oom-transient) {}
set ::FAULTSIM(oom-persistent) {}
