# SPDX-License-Identifier: AGPL-3.0-only
# Copyright (C) 2026 Manuel Baesler and contributors
#
# A stand-in for the suite's `lock_common.tcl`. The suite drives three
# connections over one file, each in a process of its own on the first
# round and all three in this interpreter on the second. This harness
# holds one writer per file and as many connections over it as a file
# opens, so only the second round runs here.

proc do_multiclient_test {varname script} {
  faultsim_delete_and_reopen
  proc code1 {tcl} { uplevel #0 $tcl }
  proc code2 {tcl} { uplevel #0 $tcl }
  proc code3 {tcl} { uplevel #0 $tcl }
  code2 { sqlite3 db2 test.db }
  code3 { sqlite3 db3 test.db }
  proc sql1 {sql} { db eval $sql }
  proc sql2 {sql} { code2 [list db2 eval $sql] }
  proc sql3 {sql} { code3 [list db3 eval $sql] }
  proc csql1 {sql} { list [catch { sql1 $sql } msg] $msg }
  proc csql2 {sql} { list [catch { sql2 $sql } msg] $msg }
  proc csql3 {sql} { list [catch { sql3 $sql } msg] $msg }
  uplevel set $varname 2
  uplevel $script
  catch { code2 { db2 close } }
  catch { code3 { db3 close } }
}

proc do_multiclient_test_body {varname script} {
  do_multiclient_test $varname $script
}
