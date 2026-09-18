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

# Whether a proc this harness called is running, which no request of
# its own may be written during.
set ::calling 0

# One request over the line: the verb, how many values follow, and each
# value as its length and its bytes. The answer is `OK` and that many
# values, or `ERR` and a message, which becomes an error here.
proc harness_send {verb args} {
  if {$::calling} {
    error "this harness cannot run a statement inside a call"
  }
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
  while {[lindex $head 0] eq "CALL"} {
    set kind [lindex $head 1]
    set n [lindex $head 2]
    set vals {}
    for {set i 0} {$i < $n} {incr i} {
      set len [gets $h]
      set val [read $h $len]
      gets $h
      lappend vals [encoding convertfrom utf-8 $val]
    }
    # A proc that runs a statement of its own would write a request
    # onto the line the answer to this call is read from, so it is
    # refused rather than let past.
    set ::calling 1
    if {[catch {harness_call $kind $vals} out]} { set out 0 }
    set ::calling 0
    set b [encoding convertto utf-8 $out]
    puts $h "RET 1"
    puts $h [string length $b]
    puts $h $b
    flush $h
    set head [gets $h]
    set n [lindex $head 1]
  }
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

# The procs the collations of this file name, by collation name.
array set ::collations {}

# The procs the functions of this file name, by function name.
array set ::functions {}

# One call the engine wrote onto the line: the name of the collation or
# the function, then its values, answered by the proc the file named.
proc harness_call {kind vals} {
  set name [lindex $vals 0]
  set held [expr {$kind eq "collate" ? $::collations($name) : $::functions($name)}]
  set cmd $held
  foreach v [lrange $vals 1 end] { lappend cmd $v }
  return [uplevel #0 $cmd]
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
  # A real is written with every digit its bits carry, because
  # `tcl_precision` is 15 and the C library is handed the double
  # itself by `sqlite3_bind_double` and not the text of it.
  if {[string is double -strict $value]} { return [format %.17g $value] }
  return "'[string map {' ''} $value]'"
}

# The methods a connection answers. TCL matches the method of a command
# by any unambiguous beginning of its name, which SQLite's own files
# write as `db func` for `db function` and as `db onecolumn` for the
# same method the shorter `db one` names.
set ::methods {
  authorizer backup busy cache changes close collate collation_needed
  commit_hook complete config copy deserialize enable_load_extension
  errorcode eval exists function interrupt last_insert_rowid nullvalue
  one onecolumn preupdate profile progress restore rollback_hook
  serialize timeout total_changes trace trace_v2 transaction
  unlock_notify update_hook version wal_hook
}

# The whole name of a method, where the one written is a beginning of
# exactly one of them, and the one written otherwise.
proc whole_method {method} {
  if {[lsearch -exact $::methods $method] >= 0} { return $method }
  set found [lsearch -all -inline -glob $::methods "$method*"]
  if {[llength $found] == 1} { return [lindex $found 0] }
  return $method
}

# A connection: the command `sqlite3` makes one and names it.
proc sqlite3 {name args} {
  set file [lindex $args 0]
  if {$file eq ""} { set file ":memory:" }
  harness_send open $name $file
  proc ::$name {method args} [string map [list %N% $name] {
    set method [whole_method $method]
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
        if {$array ne ""} {
          uplevel 1 [list set ${array}(*) $names]
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
      total_changes { return [lindex [harness_send total_changes %N%] 0] }
      last_insert_rowid { return [lindex [harness_send rowid %N%] 0] }
      nullvalue { return [harness_send null %N% [lindex $args 0]] }
      errorcode { return [lindex [harness_send errorcode %N%] 0] }
      complete { return [lindex [harness_send complete %N% [lindex $args 0]] 0] }
      collate {
        set ::collations([lindex $args 0]) [lindex $args 1]
        return [harness_send collate %N% [lindex $args 0]]
      }
      function {
        set ::functions([lindex $args 0]) [lindex $args end]
        return [harness_send function %N% [lindex $args 0]]
      }
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

# The harness holds the database, the log and the journal, and the
# machine holds no file of them, so `file size` and `file exists` over
# one of those names answer out of the harness and every other name
# reaches TCL's own command.
if {[info commands ::tcl_file] eq ""} { rename file ::tcl_file }
proc file {command args} {
  set name [lindex $args 0]
  if {$command eq "size"} {
    set bytes [lindex [harness_send size $name] 0]
    if {$bytes >= 0} { return $bytes }
  }
  if {$command eq "exists"} {
    if {[lindex [harness_send exists $name] 0]} { return 1 }
  }
  return [uplevel 1 [list ::tcl_file $command {*}$args]]
}

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
  sqlite3_memory_used sqlite3_memory_highwater
  sqlite3_config sqlite3_db_config
  sqlite3_db_status sqlite3_status
  sqlite3_reset_auto_extension sqlite3_create_function sqlite3_limit
  sqlite3_extended_result_codes sqlite3_prepare
  sqlite3_prepare_v2 sqlite3_prepare_v3 sqlite3_finalize sqlite3_step
  sqlite3_column_count sqlite3_errcode sqlite3_errmsg sqlite3_bind_parameter_count
  sqlite3_enable_shared_cache sqlite3_release_memory sqlite3_db_release_memory
  sqlite3_memdebug_vfs_oom_test sqlite3_memdebug_settitle sqlite3_memdebug_fail
  sqlite3_memdebug_pending sqlite3_memdebug_log sqlite3_stmt_status
  testvfs test_syscall test_sqlite3_log
  register_wholenumber_module register_echo_module register_tclvar_module
  register_fs_module register_dbstat_vtab register_schema_module
  run_thread_tests test_cli_invocation
  test_find_cli test_find_sqldiff
  file_control_chunksize_test file_control_sizehint_test file_control_lockproxy_test
  file_control_persist_wal file_control_powersafe_overwrite file_control_vfsname
  file_control_tempfilename file_control_external_reader
  speed_trial speed_trial_init speed_trial_summary
  tcl_variable_type
  sqlite3_db_filename sqlite3_next_stmt sqlite3_stmt_readonly
  vfs_unlink_test vfs_shared_errors
  add_alignment_test_collations add_test_collate add_test_function
  add_test_utf16bin_collate autoinstall_test_functions
  sqlite3_snapshot_get sqlite3_snapshot_open sqlite3_snapshot_free
  sqlite3_wal_autocheckpoint
} {
  proc ::$cmd {args} "error \"this harness has no [set cmd]\""
}

# What a file sets and reads back, which this engine answers the same
# way for every setting: the value it was given.
proc sqlite3_db_config {args} { return [lindex $args 2] }

# What a file holds the extended error code to, which this engine has
# none of: the case is not run rather than scored against a made-up
# code.
proc verify_ex_errcode {args} {}

# What a file says about itself, which changes nothing here.
proc breakpoint {args} {}
proc do_not_use_codec {args} {}
proc database_may_be_corrupt {args} {}
proc database_never_corrupt {args} {}
proc omit_test {args} {}
proc clang_sanitize_address {args} { return 0 }

# What the suite asks of the machine and not of the library.
proc atomic_batch_write {args} { return 0 }
proc nonzero_reserved_bytes {} { return 0 }
proc working_64bit_int {} { return 1 }
proc wal_is_capable {} { return 1 }
proc presql {args} {}
proc set_test_counter {args} { return 0 }

# `test_set_config_pagecache` sizes the page cache the C library keeps,
# which this engine has none of.
proc test_set_config_pagecache {args} { return 0 }

# hexio_read, hexio_write and the three that read or write a number of
# test_hexio.c: the bytes of a file the harness holds, as capital
# hexadecimal digits.
proc hexio_read {file offset amt} {
  return [lindex [harness_send read $file $offset $amt] 0]
}
proc hexio_write {file offset data} {
  return [lindex [harness_send write $file $offset $data] 0]
}
proc hexio_get_int {args} {
  set little 0
  if {[llength $args] > 1} {
    set little 1
    set digits [lindex $args 1]
  } else {
    set digits [lindex $args 0]
  }
  binary scan [binary format H* $digits] c* bytes
  set four {0 0 0 0}
  set count [llength $bytes]
  if {$count >= 4} {
    set four [lrange $bytes 0 3]
  } else {
    set four [concat [lrange {0 0 0 0} 0 [expr {3-$count}]] $bytes]
  }
  if {$little} { set four [lreverse $four] }
  set value 0
  foreach byte $four { set value [expr {($value << 8) | ($byte & 0xff)}] }
  if {$value >= 0x80000000} { set value [expr {$value - 0x100000000}] }
  return $value
}
proc hexio_render_int16 {value} { return [format %04X [expr {$value & 0xffff}]] }
proc hexio_render_int32 {value} { return [format %08X [expr {$value & 0xffffffff}]] }

# `sqlite3_simulate_device` names the sector size and the properties of
# the device under the file, which this engine reads none of.
proc sqlite3_simulate_device {args} { return "" }

# `sqlite3_test_control_pending_byte` moves the byte-range a lock takes,
# which this engine takes none of.
proc sqlite3_test_control_pending_byte {args} { return $::sqlite_pending_byte }

# The version this engine writes into every file it makes.
proc sqlite3_libversion_number {} { return 3053004 }
proc sqlite3_libversion {} { return "3.53.4" }
proc sqlite3_sourceid {} { return "3.53.4" }

# `sqlite3_wal_checkpoint_v2 DB MODE ?NAME?`: the pragma of the same
# name, whose three columns are the three numbers the command answers.
proc sqlite3_wal_checkpoint_v2 {db {mode passive} args} {
  return [$db eval "PRAGMA wal_checkpoint = $mode"]
}

# The commands that size or count what the C library holds, which this
# engine holds none of.
proc sqlite3_soft_heap_limit {args} { return 0 }
proc sqlite3_hard_heap_limit {args} { return 0 }
proc sqlite3_shutdown {args} { return 0 }
proc sqlite3_initialize {args} { return 0 }
proc sqlite3_db_config_lookaside {args} { return 0 }
proc optimization_control {args} { return "" }
# `sqlite3_test_control` turns on what the C library keeps for its own
# tests: the internal functions, the imposter tables, the sorter's use
# of a mapped file, and a clock that fails. This engine holds none of
# them, so a case that reads one of those answers differently rather
# than ending the file.
proc sqlite3_test_control {args} { return 0 }
proc sqlite3_soft_heap_limit64 {args} { return 0 }
proc sqlite3_config_uri {args} { return 0 }
proc sqlite3_register_cksumvfs {args} { return 0 }
proc sqlite3_multiplex_initialize {args} { return 0 }

# `sqlite3_connection_pointer` answers the pointer the C library holds
# the connection at, which the commands that take one are stand-ins
# for here, so the name of the connection stands for it.
proc sqlite3_connection_pointer {name} { return $name }

# `sqlite3_table_column_metadata DB SCHEMA TABLE COLUMN`: what the
# schema says about one column. The connection stands for the pointer,
# and the schema is always `main` here.
proc sqlite3_table_column_metadata {db schema table column} {
  return [harness_send columnmeta $db $table $column]
}

# `load_static_extension` links a module of the C library into the
# connection, which this engine holds none of, so the functions the
# module carries stay missing.
proc load_static_extension {args} { return "" }
proc extra_schema_checks {args} { return 1 }
proc test_restore_config_pagecache {args} { return 0 }
proc unregister_devsim {args} { return "" }
proc translate_selftest {args} { return "" }

# sqlite_current_time of test1.c: the moment `now` names, as the
# seconds since 1970. Nought is the clock of the machine, which this
# harness has none of, so a statement that names `now` under it is
# refused.
set ::sqlite_current_time 0
trace add variable ::sqlite_current_time write harness_clock
proc harness_clock {args} { harness_send clock $::sqlite_current_time }

# save_prng_state, restore_prng_state: the state random and randomblob
# draw from next, which a test holds to draw the same words again.
proc save_prng_state {} { harness_send save_prng }
proc restore_prng_state {} { harness_send restore_prng }
# btree_varint_test START MULTIPLIER COUNT INCREMENT of test3.c: every
# value written as a varint and read back, which raises where one of them
# does not come back unchanged.
proc btree_varint_test {start mult count incr} {
  harness_send varint [expr {$start+0}] [expr {$mult+0}] \
    [expr {$count+0}] [expr {$incr+0}]
  return ""
}

# The sqlite3_mprintf_* commands of test1.c. Each argument carries the C
# type the command hands the format: i a 32-bit int, l a 64-bit one, r a
# double, h the hexadecimal digits of one, and s a string.
proc mprintf_over {format args} {
  return [lindex [harness_send mprintf $format {*}$args] 0]
}
proc printf_int {n} { return i[expr {wide($n)}] }
proc printf_real {r} { return r[format %.17g [expr {double($r)}]] }

proc sqlite3_mprintf_int {format a b c} {
  return [mprintf_over $format [printf_int $a] [printf_int $b] [printf_int $c]]
}
proc sqlite3_mprintf_int64 {format a b c} {
  return [mprintf_over $format l[expr {wide($a)}] l[expr {wide($b)}] l[expr {wide($c)}]]
}
proc sqlite3_mprintf_long {format a b c} {
  return [mprintf_over $format l[expr {wide($a)}] l[expr {wide($b)}] l[expr {wide($c)}]]
}
proc sqlite3_mprintf_str {format a b args} {
  set text n
  if {[llength $args] > 0} { set text s[lindex $args 0] }
  return [mprintf_over $format [printf_int $a] [printf_int $b] $text]
}
proc sqlite3_mprintf_double {format a b r} {
  return [mprintf_over $format [printf_int $a] [printf_int $b] [printf_real $r]]
}
proc sqlite3_mprintf_scaled {format a b} {
  return [mprintf_over $format [printf_real [expr {double($a)*double($b)}]]]
}
proc sqlite3_mprintf_stronly {format text} {
  return [mprintf_over $format s$text]
}
proc sqlite3_mprintf_hexdouble {format hex} {
  return [mprintf_over $format h$hex]
}
# sqlite3_snprintf writes SIZE bytes counting the byte that ends the
# text, so the answer holds SIZE-1 characters at most.
proc sqlite3_snprintf_int {size format a} {
  return [string range [mprintf_over $format [printf_int $a]] 0 [expr {$size-2}]]
}
proc sqlite3_snprintf_str {size format a b args} {
  set text n
  if {[llength $args] > 0} { set text s[lindex $args 0] }
  set whole [mprintf_over $format [printf_int $a] [printf_int $b] $text]
  return [string range $whole 0 [expr {$size-2}]]
}

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

# The rows of a statement as name and value one after another, which
# `execsql2` of the suite's own tester answers.
proc execsql2 {sql {db db}} {
  set out {}
  $db eval $sql row {
    foreach name $row(*) { lappend out $name $row($name) }
  }
  return $out
}

# How long a statement took, which changes nothing this harness scores.
proc execsql_timed {sql {db db}} { return [uplevel 1 [list $db eval $sql]] }
proc do_timed_execsql_test {name sql {expected {}}} {
  uplevel 1 [list do_test $name [list execsql $sql] [list {*}$expected]]
}

# A list written again as a list, which drops the whitespace a file
# wrote it with.
proc normalize_list {L} {
  set out [list]
  foreach item $L { lappend out $item }
  return $out
}

# A real as the suite writes it: the exponent without its leading
# noughts and without the `.0` before it.
proc realnum_normalize {r} {
  string map {1.#INF inf Inf inf .0e e} [regsub -all {(e[+-])0+} $r {\1}]
}

# A path as the suite writes it, which on this platform is the path.
proc filepath_normalize {p} { return $p }
proc do_filepath_test {name script expected} {
  uplevel 1 [list do_test $name $script [filepath_normalize $expected]]
}
proc is_relative_file {file} { return [expr {[file pathtype $file] ne "absolute"}] }
proc get_pwd {} { return [pwd] }

# What a run says about itself. This harness runs one permutation, the
# one the suite's own tester runs without a name.
proc permutation {} { return "" }
proc isquick {} { return 0 }
proc verbose {} { return 0 }
proc output1 {args} {}
proc output2 {args} {}
proc output2_if_no_verbose {args} {}
proc warning {msg {append 1}} {}
proc incr_ntest {} {}
proc fail_test {name} {}

# Write-ahead logging, which a run this harness makes reads under the
# journal mode the file sets and not under a permutation.
proc wal_is_wal_mode {} { return 0 }
proc wal_set_journal_mode {{db db}} {}
proc wal_check_journal_mode {name {db db}} {}

# Whether this engine has what a name says, which `ifcapable` reads the
# same way.
proc capable {expr} { return [lindex [harness_send capable $expr] 0] }

# The database written again beside itself and read back, which a file
# calls around a run it wants undone. This harness holds one database
# per path, so a save copies the path and not the files beside it.
proc db_save {} { harness_send copy test.db sv_test.db }
proc db_restore {} { harness_send copy sv_test.db test.db }
proc db_save_and_close {} { db_save ; catch { db close } ; return "" }
proc db_restore_and_reopen {{file test.db}} {
  catch { db close }
  db_restore
  sqlite3 db $file
}

# Every row of every table taken out, and every index a statement made
# dropped, which a file calls between two runs of its own.
proc delete_all_data {} {
  foreach t [db eval {SELECT tbl_name FROM sqlite_master WHERE type='table'}] {
    catch { db eval "DELETE FROM '[string map {' ''} $t]'" }
  }
}
proc drop_all_indexes {{db db}} {
  foreach i [$db eval {
    SELECT name FROM sqlite_master WHERE type='index' AND sql LIKE 'create%'
  }] {
    catch { $db eval "DROP INDEX '[string map {' ''} $i]'" }
  }
}

# `tcl_precision` is what SQLite's own tester sets, so a real a file
# counts with is written with the digits the file's answers hold.
# sqlite_pending_byte of test2.c: where the byte-range a lock takes
# begins, which the format holds one page for.
set ::sqlite_pending_byte 0x40000000

# The limits of src/sqliteLimit.h, which a test reads to skip a case its
# build cannot reach. Each one is this engine's own where the engine
# holds a limit of its own, and the default of the header otherwise.
set ::SQLITE_MAX_LENGTH 1000000000
set ::SQLITE_MAX_SQL_LENGTH 1000000000
set ::SQLITE_MAX_COLUMN 2000
set ::SQLITE_MAX_EXPR_DEPTH 200
set ::SQLITE_MAX_COMPOUND_SELECT 500
set ::SQLITE_MAX_VDBE_OP 250000000
set ::SQLITE_MAX_FUNCTION_ARG 1000
set ::SQLITE_MAX_ATTACHED 10
set ::SQLITE_MAX_VARIABLE_NUMBER 32766
set ::SQLITE_MAX_PAGE_SIZE 65536
set ::SQLITE_MAX_PAGE_COUNT 4294967294
set ::SQLITE_MAX_LIKE_PATTERN_LENGTH 50000
set ::SQLITE_MAX_TRIGGER_DEPTH 1000
set ::SQLITE_MAX_MMAP_SIZE 0
set ::SQLITE_DEFAULT_FILE_FORMAT 4
set ::AUTOVACUUM 0
set ::TEMP_STORE 1
set ::bitmask_size 64

# The compile options a file reads to skip a case its build cannot
# reach, which `ifcapable` answers for as well.
foreach option {
  fts3 fts5 rtree icu vtab incrblob shared_cache codec atomicwrite vacuum
  attach explain autovacuum session setlk_timeout configslower
  memorymanage threadsafe
} { set ::sqlite_options($option) 0 }
foreach option {
  wal utf16 integrityck casesensitivelike trigger view subquery compound
  foreignkey json1 like_match_blobs pragma reindex analyze altertable
  cast check conflict datetime floatingpoint or_opt stat4 update_delete_limit
} { set ::sqlite_options($option) 1 }
set ::sqlite_options(default_autovacuum) 0

# What the suite's own tester takes off the command line, which this
# harness takes none of: the defaults of `tester.tcl` lines 378 to 391.
array set ::cmdlinearg {
  soft-heap-limit 0
  hard-heap-limit 0
  maxerror 1000
  malloctrace 0
  backtrace 10
  binarylog 0
  soak 0
  file-retries 0
  file-retry-delay 0
  start {}
  match {}
  verbose {}
  output {}
  testdir testdir
}
set ::SQLITE_MAX_WORKER_THREADS 0

set ::tcl_precision 15

# The connection every file reads without opening one, which SQLite's
# own tester opens the same way.
sqlite3 db test.db
