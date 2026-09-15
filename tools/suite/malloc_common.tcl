# SPDX-License-Identifier: AGPL-3.0-only
# Copyright (C) 2026 Manuel Baesler and contributors
#
# A stand-in for the suite's `malloc_common.tcl`: every command in it
# asks the C library to fail an allocation, which this harness cannot.

proc do_malloc_test {args} {}
proc do_faultsim_test {args} {}
proc do_one_faultsim_test {args} {}
proc faultsim_save {args} {}
proc faultsim_save_and_close {} {}
proc faultsim_restore {args} {}
proc faultsim_restore_and_reopen {args} { reset_db }
proc faultsim_delete_and_reopen {args} { reset_db }
proc faultsim_integrity_check {args} {}
proc faultsim_test_result {args} {}
proc faultsim_test_control {args} {}
proc do_write_test {args} {}
proc sqlite3_memdebug_vfs_oom_test {args} { return 0 }
set ::FAULTSIM(oom-transient) {}
set ::FAULTSIM(oom-persistent) {}
