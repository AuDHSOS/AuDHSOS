# SPDX-License-Identifier: AGPL-3.0-only
# Copyright (C) 2026 Manuel Baesler and contributors
#
# A stand-in for the suite's `lock_common.tcl`: it drives three
# processes over one file, and this harness holds one.

proc do_multiclient_test {varname script} {}
proc code1 {script} { uplevel 1 $script }
proc code2 {script} {}
proc code3 {script} {}
proc sql1 {sql} { return [db eval $sql] }
proc sql2 {sql} { return {} }
proc sql3 {sql} { return {} }
proc csql1 {sql} { return [catchsql $sql] }
proc csql2 {sql} { return {0 {}} }
proc csql3 {sql} { return {0 {}} }
proc do_multiclient_test_body {args} {}
