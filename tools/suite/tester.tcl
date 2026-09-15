# SPDX-License-Identifier: AGPL-3.0-only
# Copyright (C) 2026 Manuel Baesler and contributors
#
# The commands a file of SQLite's own suite drives, written to reach
# `db-sqlite` over the line the runner opened. SQLite's own tester.tcl
# is never read: it calls eighty commands `testfixture` links in, which
# are the C library's internals.
#
# `docs/17-the-suite-on-the-machine.md` says what this answers and why.

set ::testprefix ""
set ::nErr 0

# One request over the line: the verb, how many values follow, and each
# value as its length and its bytes. The answer is `OK` and that many
# values, or `ERR` and a message, which becomes an error here.
proc harness_send {verb args} {
  set h $::harness
  puts $h "REQ $verb [llength $args]"
  foreach a $args {
    set b [encoding convertto utf-8 $a]
    puts $h [string length $b]
    puts $h $b
  }
  flush $h
  set head [gets $h]
  set n [lindex $head 1]
  if {[lindex $head 0] eq "ERR"} {
    set msg [encoding convertfrom utf-8 [read $h $n]]
    gets $h
    error $msg
  }
  set out {}
  for {set i 0} {$i < $n} {incr i} {
    set len [gets $h]
    set val [read $h $len]
    gets $h
    lappend out [encoding convertfrom utf-8 $val]
  }
  return $out
}

# What the TCL interface binds: `$name`, `$name(key)`, `:name` and
# `@name` inside a statement stand for the variable of that name, which
# this writes into the statement as a literal. A parameter inside a
# string, a comment or an identifier in brackets is text and is left.
proc bound {sql} {
  set out ""
  set n [string length $sql]
  for {set i 0} {$i < $n} {incr i} {
    set c [string index $sql $i]
    if {$c eq "'" || $c eq "\"" || $c eq "\[" || $c eq "`"} {
      set close [expr {$c eq "\[" ? "\]" : $c}]
      append out $c
      incr i
      while {$i < $n} {
        set d [string index $sql $i]
        append out $d
        incr i
        if {$d eq $close} break
      }
      incr i -1
      continue
    }
    if {$c eq "-" && [string index $sql $i+1] eq "-"} {
      while {$i < $n && [string index $sql $i] ne "\n"} {
        append out [string index $sql $i]
        incr i
      }
      incr i -1
      continue
    }
    if {$c ne "\$" && $c ne ":" && $c ne "@"} {
      append out $c
      continue
    }
    # `::` is a name and not a parameter, and a `:` that no name
    # follows is the text it stands as.
    set j [expr {$i+1}]
    set name ""
    if {$c eq "\$" && [string index $sql $j] eq ":" && [string index $sql $j+1] eq ":"} {
      append name "::"
      incr j 2
    }
    while {$j < $n && [string match {[A-Za-z0-9_]} [string index $sql $j]]} {
      append name [string index $sql $j]
      incr j
    }
    if {$name eq "" || $name eq "::"} {
      append out $c
      continue
    }
    if {[string index $sql $j] eq "("} {
      set k [string first ")" $sql $j]
      if {$k >= 0} {
        append name [string range $sql $j $k]
        set j [expr {$k+1}]
      }
    }
    append out [literal $name]
    set i [expr {$j-1}]
  }
  return $out
}

# One variable as the literal the interface binds it as: a whole number
# and a real as themselves, anything else as text, and a variable that
# holds nothing as `NULL`.
proc literal {name} {
  upvar #0 $name global_value
  set found 0
  set value ""
  if {[uplevel 3 [list info exists $name]]} {
    set value [uplevel 3 [list set $name]]
    set found 1
  } elseif {[info exists ::$name]} {
    set value [set ::$name]
    set found 1
  }
  if {!$found} { return "NULL" }
  if {[string is entier -strict $value]} { return $value }
  if {[string is double -strict $value]} { return $value }
  return "'[string map {' ''} $value]'"
}

# A connection: the command `sqlite3` makes one and names it.
proc sqlite3 {name args} {
  set file [lindex $args 0]
  if {$file eq ""} { set file ":memory:" }
  harness_send open $name $file
  proc ::$name {method args} [string map [list %N% $name] {
    switch -exact -- $method {
      eval {
        set sql [bound [lindex $args 0]]
        if {[llength $args] == 1} {
          return [harness_send eval %N% $sql]
        }
        # `eval SQL SCRIPT` names each column as a variable of its own;
        # `eval SQL ARRAY SCRIPT` names them in an array, with `*`
        # holding the names.
        if {[llength $args] == 2} {
          set array ""
          set body [lindex $args 1]
        } else {
          set array [lindex $args 1]
          set body [lindex $args 2]
        }
        set names [harness_send names %N% $sql]
        set rows [harness_send eval %N% $sql]
        set w [llength $names]
        if {$w == 0} {
          harness_send eval %N% $sql
          return {}
        }
        set at 0
        while {$at < [llength $rows]} {
          for {set c 0} {$c < $w} {incr c} {
            set value [lindex $rows [expr {$at+$c}]]
            if {$array eq ""} {
              uplevel 1 [list set [lindex $names $c] $value]
            } else {
              uplevel 1 [list set ${array}([lindex $names $c]) $value]
            }
          }
          if {$array ne ""} {
            uplevel 1 [list set ${array}(*) $names]
          }
          incr at $w
          uplevel 1 $body
        }
        return {}
      }
      one - onecolumn { return [lindex [harness_send eval %N% [bound [lindex $args 0]]] 0] }
      exists { return [expr {[llength [harness_send eval %N% [bound [lindex $args 0]]]] > 0}] }
      close { return [harness_send close %N%] }
      changes { return [lindex [harness_send changes %N%] 0] }
      total_changes { return [lindex [harness_send changes %N%] 0] }
      last_insert_rowid { return [lindex [harness_send rowid %N%] 0] }
      nullvalue { return [harness_send null %N% [lindex $args 0]] }
      errorcode { return [lindex [harness_send errorcode %N%] 0] }
      complete { return 1 }
      transaction {
        harness_send eval %N% BEGIN
        set rc [catch { uplevel 1 [lindex $args end] } msg]
        harness_send eval %N% [expr {$rc ? "ROLLBACK" : "COMMIT"}]
        if {$rc} { error $msg }
        return $msg
      }
      copy - authorizer - busy - cache - collate - collation_needed -
      commit_hook - enable_load_extension - function - interrupt -
      preupdate - profile - progress - rollback_hook - timeout -
      trace - trace_v2 - unlock_notify - update_hook - version -
      wal_hook - config - deserialize - serialize - backup - restore {
        return {}
      }
      default { error "no such method: $method" }
    }
  }]
  return $name
}

proc execsql {sql {db db}} { return [$db eval $sql] }
proc catchsql {sql {db db}} {
  set rc [catch { $db eval $sql } msg]
  return [list $rc $msg]
}
proc db_eval {sql} { return [db eval $sql] }
proc stepsql {db sql} { return [$db eval $sql] }

proc forcedelete {args} { foreach f $args { harness_send delete $f } }
proc delete_file {args} { foreach f $args { harness_send delete $f } }
proc forcecopy {from to} { harness_send copy $from $to }
proc copy_file {from to} { harness_send copy $from $to }
proc file_exists {f} { return [lindex [harness_send exists $f] 0] }

proc reset_db {} {
  catch { db close }
  forcedelete test.db test.db-journal test.db-wal test.db-shm
  sqlite3 db test.db
}
proc db_delete_and_reopen {{file test.db}} { reset_db }
proc drop_all_tables {{db db}} {
  foreach t [$db eval {SELECT name FROM sqlite_master WHERE type='table' AND name NOT LIKE 'sqlite_%'}] {
    catch { $db eval "DROP TABLE '$t'" }
  }
}

proc finish_test {} { harness_send done ; exit 0 }
proc finalize_testing {} { finish_test }

# A case: its name, the script it runs, and what the file says the
# script answers. What the file says is read as `do_test` of the
# suite's own tester reads it: `/RE/` is a regular expression, `~/RE/`
# one that must not match, `#/A..B/` a range, `*GLOB*` a pattern, and
# anything else the text itself.
proc fix_testname {varname} {
  upvar $varname testname
  if {[info exists ::testprefix] && $::testprefix ne ""
   && [string is digit [string range $testname 0 0]]
  } {
    set testname "${::testprefix}-$testname"
  }
}

proc matches {result expected} {
  if {[regexp {^[~#]?/.*/$} $expected]} {
    if {[string index $expected 0] eq "~"} {
      set re [string range $expected 2 end-1]
      if {[string index $re 0] eq "*"} {
        return [expr {![string match $re $result]}]
      }
      set re [string map {# {[-0-9.]+}} $re]
      return [expr {![regexp $re $result]}]
    }
    if {[string index $expected 0] eq "#"} {
      set e2 [string range $expected 2 end-1]
      set ok 1
      foreach i $result j $e2 {
        if {[regexp {^(-?\d+)\.\.(-?\d+)$} $j all A B]} {
          set ok [expr {$i+0>=$A && $i+0<=$B}]
        } else {
          set ok [expr {$i+0>=0.9*$j && $i+0<=1.1*$j}]
        }
        if {!$ok} break
      }
      if {$ok && [llength $result]!=[llength $e2]} { set ok 0 }
      return $ok
    }
    set re [string range $expected 1 end-1]
    if {[string index $re 0] eq "*"} {
      return [string match $re $result]
    }
    set re [string map {# {[-0-9.]+}} $re]
    return [regexp $re $result]
  }
  if {[regexp {^~?\*.*\*$} $expected]} {
    if {[string index $expected 0] eq "~"} {
      return [expr {![string match [string range $expected 1 end] $result]}]
    }
    return [string match $expected $result]
  }
  if {[string compare $result $expected] == 0} { return 1 }
  return [fpnum_compare $result $expected]
}

# `fpnum_compare` of `src/test1.c`: the two texts are whitespace-parted
# tokens, a token that is not a number matches exactly, and a token
# that is a number matches to fifteen significant digits.
proc fpnum_compare {a b} {
  set ta [regexp -all -inline {\S+} $a]
  set tb [regexp -all -inline {\S+} $b]
  if {[llength $ta] != [llength $tb]} { return 0 }
  foreach x $ta y $tb {
    if {$x eq $y} continue
    if {![string is double -strict $x] || ![string is double -strict $y]} { return 0 }
    if {[format %.15g [expr {double($x)}]] ne [format %.15g [expr {double($y)}]]} { return 0 }
  }
  return 1
}

proc do_test {name script expected} {
  fix_testname name
  set rc [catch { uplevel 1 $script } result]
  if {$rc} {
    harness_send case $name refused $result
  } elseif {[matches $result $expected]} {
    harness_send case $name passed
  } else {
    harness_send case $name failed $result $expected
  }
}

proc do_execsql_test {args} {
  set db db
  if {[lindex $args 0] eq "-db"} {
    set db [lindex $args 1]
    set args [lrange $args 2 end]
  }
  if {[llength $args] == 2} {
    foreach {testname sql} $args {}
    set result ""
  } elseif {[llength $args] == 3} {
    foreach {testname sql result} $args {}
    if {[llength $result] == 0} { set result "" }
  } else {
    error "wrong # args: should be \"do_execsql_test ?-db DB? testname sql ?result?\""
  }
  uplevel do_test [list $testname] [list "execsql {$sql} $db"] [list [list {*}$result]]
}

proc do_catchsql_test {testname sql result} {
  uplevel do_test [list $testname] [list "catchsql {$sql}"] [list $result]
}
proc do_realnum_test {name script expected} {
  uplevel 1 [list do_test $name $script $expected]
}
proc do_execsql_test_alt {args} {}

proc ifcapable {expr code {maybe_else ""} {else_code ""}} {
  if {[lindex [harness_send capable $expr] 0]} {
    uplevel 1 $code
  } elseif {$maybe_else eq "else"} {
    uplevel 1 $else_code
  }
}

proc integrity_check {name {db db}} {
  uplevel 1 [list do_test $name [list execsql {PRAGMA integrity_check} $db] {ok}]
}
proc execpresql {args} {}
proc explain {args} {}
proc explain_i {args} {}
proc explain_no_trace {args} { return {} }
proc do_eqp_test {args} {}
proc do_select_tests {args} {}
proc query_plan {args} { return {} }
proc memdebug_log_sql {args} {}

# What `testfixture` links in and this repository does not have. A case
# that reads the answer of one of these is refused, because the command
# raises rather than answering nothing.
foreach cmd {
  sqlite3_test_control sqlite3_test_control_pending_byte sqlite3_soft_heap_limit
  sqlite3_hard_heap_limit sqlite3_memory_used sqlite3_memory_highwater
  sqlite3_shutdown sqlite3_initialize sqlite3_config sqlite3_db_config
  sqlite3_db_config_lookaside sqlite3_db_status sqlite3_status
  sqlite3_reset_auto_extension sqlite3_create_function sqlite3_limit
  sqlite3_extended_result_codes sqlite3_connection_pointer sqlite3_prepare
  sqlite3_prepare_v2 sqlite3_prepare_v3 sqlite3_finalize sqlite3_step
  sqlite3_column_count sqlite3_errcode sqlite3_errmsg sqlite3_bind_parameter_count
  sqlite3_enable_shared_cache sqlite3_release_memory sqlite3_db_release_memory
  sqlite3_sourceid sqlite3_libversion sqlite3_libversion_number
  sqlite3_memdebug_vfs_oom_test sqlite3_memdebug_settitle sqlite3_memdebug_fail
  sqlite3_memdebug_pending sqlite3_memdebug_log sqlite3_stmt_status
  testvfs test_syscall test_sqlite3_log optimization_control
  register_wholenumber_module register_echo_module register_tclvar_module
  register_fs_module register_dbstat_vtab register_schema_module
  breakpoint do_not_use_codec database_may_be_corrupt database_never_corrupt
  load_static_extension permutation run_thread_tests test_cli_invocation
  test_find_cli test_find_sqldiff test_set_config_pagecache
  file_control_chunksize_test file_control_sizehint_test file_control_lockproxy_test
  file_control_persist_wal file_control_powersafe_overwrite file_control_vfsname
  file_control_tempfilename file_control_external_reader
  speed_trial speed_trial_init speed_trial_summary
  clang_sanitize_address tcl_variable_type omit_test
  sqlite3_db_filename sqlite3_next_stmt sqlite3_stmt_readonly
  sqlite3_table_column_metadata vfs_unlink_test vfs_shared_errors
  add_alignment_test_collations add_test_collate add_test_function
  add_test_utf16bin_collate autoinstall_test_functions
  sqlite3_snapshot_get sqlite3_snapshot_open sqlite3_snapshot_free
  sqlite3_wal_checkpoint_v2 sqlite3_wal_autocheckpoint
} {
  proc ::$cmd {args} "error \"this harness has no [set cmd]\""
}

# What the suite asks of the machine and not of the library.
proc atomic_batch_write {args} { return 0 }
proc nonzero_reserved_bytes {} { return 0 }
proc working_64bit_int {} { return 1 }
proc wal_is_capable {} { return 1 }
proc presql {args} {}
proc set_test_counter {args} { return 0 }
proc crashsql {args} { error "this harness does not crash" }
proc do_faultsim_test {args} {}
proc do_malloc_test {args} {}
proc do_ioerr_test {args} {}
proc faultsim_save_and_close {} {}
proc faultsim_restore_and_reopen {} { reset_db }
proc faultsim_delete_and_reopen {args} { reset_db }
proc faultsim_integrity_check {args} {}
proc faultsim_test_result {args} {}
proc faultsim_test_control {args} {}

# `tcl_precision` is what SQLite's own tester sets, so a real a file
# counts with is written with the digits the file's answers hold.
set ::tcl_precision 15

# The connection every file reads without opening one, which SQLite's
# own tester opens the same way.
sqlite3 db test.db
