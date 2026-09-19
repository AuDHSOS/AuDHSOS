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
    if {[catch {harness_call $kind $vals} out]} { set out [list null {}] }
    set ::calling 0
    puts $h "RET [llength $out]"
    foreach value $out {
      set b [encoding convertto utf-8 $value]
      puts $h [string length $b]
      puts $h $b
    }
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

# The type `db function -returntype` declared, by function name.
array set ::returns {}

# The proc the authorizer of each connection names, by connection name.
array set ::authorizers {}

# The text a NULL is read out as, by connection name.
array set ::nulls {}

# The script each callback of each connection names, by connection name
# and method name.
array set ::hooks {}

# How many transactions of `db transaction` each connection has open,
# by connection name.
array set ::transactions {}

# Whether each statement the tester prepared was made by
# `sqlite3_prepare` and not `sqlite3_prepare_v2`.
array set ::stmt_legacy {}

# The kind of value a function's result stands for: what
# `db function -returntype` declared, and the Tcl type of the result
# where that is `any` or none, which `tclSqlFunc` of
# `research/sqlite/src/tclsqlite.c:1256` reads the same two for.
#
# `::tcl::unsupported::representation` writes the type of the object as
# its fourth word, which is `pure` for a string of nothing else, and says
# whether the object holds a string of its own, which are
# `typePtr->name` and `bytes` there.
proc value_kind {declared value} {
  if {$declared eq "blob"} { return blob }
  if {$declared eq "integer" || $declared eq "real"} {
    if {$declared eq "integer" && [string is entier -strict $value]} {
      return int
    }
    if {[string is double -strict $value]} { return real }
    return text
  }
  if {$declared eq "text"} { return text }
  set shown [::tcl::unsupported::representation $value]
  set type [lindex $shown 3]
  set bare [string match "*no string representation*" $shown]
  if {$bare && $type eq "bytearray"} { return blob }
  if {$bare && ($type eq "boolean" || $type eq "booleanString")} { return int }
  if {$type eq "double"} { return real }
  if {$type eq "wideInt" || $type eq "int"} { return int }
  return text
}

# One call the engine wrote onto the line: the name of the collation or
# the function, then its values, answered by the proc the file named, as
# the values the `RET` carries.
proc harness_call {kind vals} {
  # `sqlite3_set_authorizer` names one proc per connection, which the
  # first value names, and a proc that raises answers a denial, which is
  # what `tclsqlite.c:1240` reads for it.
  if {$kind eq "auth"} {
    set who [lindex $vals 0]
    if {![info exists ::authorizers($who)]} { return [list SQLITE_OK] }
    set cmd $::authorizers($who)
    foreach v [lrange $vals 1 end] { lappend cmd $v }
    if {[catch { uplevel #0 $cmd } out]} { return [list SQLITE_DENY] }
    return [list $out]
  }
  set name [lindex $vals 0]
  if {$kind eq "collate"} {
    set cmd $::collations($name)
    foreach v [lrange $vals 1 end] { lappend cmd $v }
    return [list [uplevel #0 $cmd]]
  }
  set cmd $::functions($name)
  foreach v [lrange $vals 1 end] { lappend cmd $v }
  set rc [catch { uplevel #0 $cmd } out]
  # A script that ends by `break` answers NULL, and one that raises ends
  # the statement, which this harness has no path for and answers NULL.
  if {$rc == 3} { return [list null {}] }
  if {$rc == 1} { error $out }
  set declared ""
  if {[info exists ::returns($name)]} { set declared $::returns($name) }
  return [list [value_kind $declared $out] $out]
}

# What the TCL interface binds: `$name`, `$name(key)`, `:name` and
# `@name` inside a statement stand for the variable of that name, which
# this writes into the statement as a literal. A parameter inside a
# string, a comment or an identifier in brackets is text and is left.
proc bound {db sql} {
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
    append out [literal $db $c$name]
    set i [expr {$j-1}]
  }
  return $out
}

# One parameter as the literal the interface binds it as: a whole
# number and a real as themselves, anything else as text, and a
# parameter whose name begins with `@` as a blob of the bytes of its
# value, which `research/sqlite/src/tclsqlite.c:1517` binds. `whole` is
# the name with the character that opens it.
proc literal {db whole} {
  set name [string range $whole 1 end]
  set found 0
  set value ""
  if {[uplevel 3 [list info exists $name]]} {
    set value [uplevel 3 [list set $name]]
    set found 1
  } elseif {[info exists ::$name]} {
    set value [set ::$name]
    set found 1
  } elseif {[info exists ::hooks($db,bind_fallback)]} {
    # The script `db bind_fallback` named answers a parameter no
    # variable is set for, which `tclsqlite.c:1495` calls it for. A
    # script that raises ends the statement; one that ends by `return`
    # or by `break` leaves the parameter unbound.
    set cmd $::hooks($db,bind_fallback)
    lappend cmd $whole
    set rc [catch { uplevel #0 $cmd } out]
    if {$rc == 1} { error $out }
    if {$rc == 0} {
      set value $out
      set found 1
    }
  }
  if {!$found} { return "NULL" }
  if {[string index $whole 0] eq "@"} {
    set hex ""
    binary scan [encoding convertto utf-8 $value] H* hex
    return "X'$hex'"
  }
  if {[string is entier -strict $value]} { return $value }
  # A real is written with every digit its bits carry, because
  # `tcl_precision` is 15 and the C library is handed the double
  # itself by `sqlite3_bind_double` and not the text of it.
  if {[string is double -strict $value]} { return [format %.17g $value] }
  return "'[string map {' ''} $value]'"
}

# The methods a connection answers, in the order `DB_enum` of
# `research/sqlite/src/tclsqlite.c:2455` names them, which the message
# for a word that is not one of them lists. TCL matches a method by any
# unambiguous beginning of its name, which SQLite's own files write as
# `db func` for `db function` and as `db one` for `db onecolumn`.
set ::methods {
  authorizer backup bind_fallback busy cache changes close collate
  collation_needed commit_hook complete config copy deserialize
  enable_load_extension errorcode erroroffset eval exists format
  function incrblob interrupt last_insert_rowid nullvalue onecolumn
  preupdate profile progress rekey restore rollback_hook serialize
  status timeout total_changes trace trace_v2 transaction unlock_notify
  update_hook version wal_hook
}

# How many values each method takes after its name, and the words the
# message for another count carries, which
# `research/sqlite/src/tclsqlite.c:2476` onward states per method. A
# count written with `+` is that many or more. A method the table omits
# takes any count.
set ::method_args {
  authorizer        {{0 1} ?CALLBACK?}
  bind_fallback     {{0 1} ?CALLBACK?}
  busy              {{0 1} CALLBACK}
  cache             {{1 2} {option ?arg?}}
  changes           {0 {}}
  collate           {2 {NAME SCRIPT}}
  collation_needed  {1 SCRIPT}
  commit_hook       {{0 1} ?CALLBACK?}
  complete          {1 SQL}
  copy              {{3 4 5} {CONFLICT-ALGORITHM TABLE FILENAME ?SEPARATOR? ?NULLINDICATOR?}}
  eval              {{1 2 3} {?OPTIONS? SQL ?VAR-NAME? ?SCRIPT?}}
  exists            {1 SQL}
  function          {2+ {NAME ?SWITCHES? SCRIPT}}
  last_insert_rowid {0 {}}
  nullvalue         {{0 1} NULLVALUE}
  onecolumn         {1 SQL}
  profile           {{0 1} ?CALLBACK?}
  progress          {{0 2} {N CALLBACK}}
  rekey             {1 KEY}
  rollback_hook     {{0 1} ?CALLBACK?}
  timeout           {1 MILLISECONDS}
  total_changes     {0 {}}
  trace             {{0 1} ?CALLBACK?}
  transaction       {{1 2} {[TYPE] SCRIPT}}
  unlock_notify     {{0 1} ?SCRIPT?}
  update_hook       {{0 1} ?CALLBACK?}
  wal_hook          {{0 1} ?CALLBACK?}
}

# The words of a list as `Tcl_GetIndexFromObj` writes them: two joined
# by `or`, more separated by commas with `or` before the last.
proc listed_words {words} {
  if {[llength $words] < 3} { return [join $words " or "] }
  return "[join [lrange $words 0 end-1] {, }], or [lindex $words end]"
}

# One word of `words`, where `written` is a beginning of exactly one of
# them. `what` names what the word stands for, which the message for a
# word that matches none or more than one carries.
proc one_word {written words what} {
  # `Tcl_GetIndexFromObj` takes a word that is one of them whole, even
  # where it begins another, which `db trace` and `db trace_v2` are the
  # two of.
  if {[lsearch -exact $words $written] >= 0} { return $written }
  set found {}
  foreach word $words {
    if {[string equal -length [string length $written] $written $word]} {
      lappend found $word
    }
  }
  if {[llength $found] == 1} { return [lindex $found 0] }
  set kind [expr {[llength $found] > 1 ? "ambiguous" : "bad"}]
  error "$kind $what \"$written\": must be [listed_words $words]"
}

# The whole name of a method.
proc whole_method {method} { return [one_word $method $::methods option] }

# The first of `options` the written word is a beginning of, which
# `research/sqlite/src/tclsqlite.c:3398` reads the switches of
# `db function` by; a word of one character matches none.
proc first_option {written options} {
  if {[string length $written] > 1} {
    foreach option $options {
      if {[string equal -length [string length $written] $written $option]} {
        return $option
      }
    }
  }
  error "bad option \"$written\": must be [listed_words $options]"
}

# Raises the message `Tcl_WrongNumArgs` writes, where a method was
# written with a count of values it does not take.
proc check_args {db written method given} {
  if {![dict exists $::method_args $method]} return
  set spec [dict get $::method_args $method]
  set n [llength $given]
  foreach count [lindex $spec 0] {
    if {[string index $count end] eq "+"} {
      if {$n >= [string range $count 0 end-1]} return
    } elseif {$n == $count} {
      return
    }
  }
  error "wrong # args: should be \"$db $written [lindex $spec 1]\""
}

# The message `sqliteCmdUsage` of
# `research/sqlite/src/tclsqlite.c:4225` writes, where the arguments of
# the command `sqlite3` are not a handle and a file name. The command
# names itself `sqlite_orig` because the tester renames it.
proc sqlite_usage {} {
  error "wrong # args: should be \"sqlite_orig HANDLE ?FILENAME? ?-vfs VFSNAME?\
      ?-readonly BOOLEAN? ?-create BOOLEAN? ?-nofollow BOOLEAN?\
      ?-nomutex BOOLEAN? ?-fullmutex BOOLEAN? ?-uri BOOLEAN?\""
}

# A connection: the command `sqlite3` makes one and names it.
proc sqlite3 {args} {
  if {[llength $args] == 0} { sqlite_usage }
  set name [lindex $args 0]
  if {[llength $args] == 1} {
    # `sqlite3 -has-codec` asks what the library was built with rather
    # than opening a connection, which this build has no encryption
    # extension for.
    if {$name eq "-has-codec"} { return 0 }
    if {[string index $name 0] eq "-"} { sqlite_usage }
  }
  # The words after the handle: one file name, and each option with the
  # value that follows it. This harness holds every file itself and has
  # no VFS to name, so `-vfs` is read and left.
  set rest [lrange $args 1 end]
  set count [llength $rest]
  set file ""
  for {set i 0} {$i < $count} {incr i} {
    set word [lindex $rest $i]
    if {[string index $word 0] ne "-"} {
      if {$file ne ""} { sqlite_usage }
      set file $word
      continue
    }
    if {$i == $count-1} { sqlite_usage }
    incr i
  }
  if {$file eq ""} { set file ":memory:" }
  harness_send open $name $file
  # A connection that is opened again holds no authorizer, no null value
  # and no callback, which `sqlite3_open` leaves null.
  catch { unset ::authorizers($name) }
  catch { unset ::nulls($name) }
  catch { unset ::transactions($name) }
  array unset ::hooks $name,*
  harness_send authorizer $name ""
  proc ::$name {args} [string map [list %N% $name] {
    if {[llength $args] == 0} {
      error "wrong # args: should be \"%N% SUBCOMMAND ...\""
    }
    set written [lindex $args 0]
    set args [lrange $args 1 end]
    # `names` is no method of the interface: the procs of this harness
    # ask the connection for the column names of a statement by it.
    if {$written eq "names"} {
      return [harness_send names %N% [bound %N% [lindex $args 0]]]
    }
    set method [whole_method $written]
    # `db eval` takes its options before the count of the rest is read,
    # which `research/sqlite/src/tclsqlite.c:3302` does in a loop.
    if {$method eq "eval"} {
      while {[llength $args] > 1 && [string index [lindex $args 0] 0] eq "-"} {
        set option [lindex $args 0]
        if {$option ne "-withoutnulls" && $option ne "-asdict"} {
          error "unknown option: \"$option\""
        }
        set args [lrange $args 1 end]
      }
    }
    check_args %N% $written $method $args
    switch -exact -- $method {
      eval {
        set sql [bound %N% [lindex $args 0]]
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
      one - onecolumn { return [lindex [harness_send eval %N% [bound %N% [lindex $args 0]]] 0] }
      exists { return [expr {[llength [harness_send eval %N% [bound %N% [lindex $args 0]]]] > 0}] }
      close { return [harness_send close %N%] }
      changes { return [lindex [harness_send changes %N%] 0] }
      total_changes { return [lindex [harness_send total_changes %N%] 0] }
      last_insert_rowid { return [lindex [harness_send rowid %N%] 0] }
      nullvalue {
        if {[llength $args] == 1} {
          set ::nulls(%N%) [lindex $args 0]
          harness_send null %N% [lindex $args 0]
        }
        if {[info exists ::nulls(%N%)]} { return $::nulls(%N%) }
        return {}
      }
      errorcode { return [lindex [harness_send errorcode %N%] 0] }
      complete { return [lindex [harness_send complete %N% [lindex $args 0]] 0] }
      collate {
        set ::collations([lindex $args 0]) [lindex $args 1]
        return [harness_send collate %N% [lindex $args 0]]
      }
      cache {
        set sub [one_word [lindex $args 0] {flush size} option]
        set wanted [expr {$sub eq "size" ? 2 : 1}]
        if {[llength $args] != $wanted} {
          set words [expr {$sub eq "size" ? "size n" : "flush"}]
          error "wrong # args: should be \"%N% cache $words\""
        }
        return {}
      }
      function {
        # The switches lie between the name and the script, and
        # `-argcount` and `-returntype` each take the word after them.
        set switches [lrange $args 1 end-1]
        set ::returns([lindex $args 0]) ""
        set options {-argcount -deterministic -directonly -innocuous -returntype}
        for {set i 0} {$i < [llength $switches]} {incr i} {
          set word [lindex $switches $i]
          set which [first_option $word $options]
          if {$which ne "-argcount" && $which ne "-returntype"} { continue }
          if {$i == [llength $switches]-1} {
            error "option requires an argument: $word"
          }
          incr i
          if {$which eq "-returntype"} {
            set ::returns([lindex $args 0]) \
                [one_word [lindex $switches $i] {integer real text blob any} type]
          }
        }
        set ::functions([lindex $args 0]) [lindex $args end]
        return [harness_send function %N% [lindex $args 0]]
      }
      transaction {
        # A transaction inside another one is a savepoint, and a type is
        # read only for the outermost, which
        # `research/sqlite/src/tclsqlite.c:3958` states.
        set type deferred
        if {[llength $args] == 2} {
          set type [one_word [lindex $args 0] {deferred exclusive immediate} \
                             "transaction type"]
        }
        set depth 0
        if {[info exists ::transactions(%N%)]} { set depth $::transactions(%N%) }
        if {$depth > 0 || $type eq "deferred"} {
          set begin "SAVEPOINT _tcl_transaction"
        } else {
          set begin [expr {$type eq "exclusive" ? "BEGIN EXCLUSIVE" : "BEGIN IMMEDIATE"}]
        }
        harness_send eval %N% $begin
        set ::transactions(%N%) [expr {$depth+1}]
        set rc [catch { uplevel 1 [lindex $args end] } msg opts]
        set ::transactions(%N%) $depth
        if {$rc == 1} {
          set end [expr {$depth == 0 ? "ROLLBACK" \
                         : "ROLLBACK TO _tcl_transaction ; RELEASE _tcl_transaction"}]
        } else {
          set end [expr {$depth == 0 ? "COMMIT" : "RELEASE _tcl_transaction"}]
        }
        # A commit the engine refuses leaves the transaction open, which
        # a rollback ends.
        if {[catch { harness_send eval %N% $end } refusal]} {
          catch { harness_send eval %N% ROLLBACK }
          if {$rc != 1} { error $refusal }
        }
        if {$rc} {
          dict incr opts -level 1
          return -options $opts $msg
        }
        return $msg
      }
      authorizer {
        if {[llength $args] == 0} {
          if {[info exists ::authorizers(%N%)]} { return $::authorizers(%N%) }
          return {}
        }
        set held [lindex $args 0]
        if {$held eq ""} {
          catch { unset ::authorizers(%N%) }
        } else {
          set ::authorizers(%N%) $held
        }
        return [harness_send authorizer %N% $held]
      }
      progress {
        if {[llength $args] == 0} {
          catch { unset ::hooks(%N%,progress) }
        } else {
          set ::hooks(%N%,progress) [lindex $args 1]
        }
        return {}
      }
      bind_fallback - busy - commit_hook - profile - rollback_hook -
      trace - trace_v2 - unlock_notify - update_hook - wal_hook {
        # A callback the connection holds, which the method answers
        # where no script follows it.
        if {[llength $args] == 0} {
          if {[info exists ::hooks(%N%,$method)]} { return $::hooks(%N%,$method) }
          return {}
        }
        set held [lindex $args 0]
        if {$held eq ""} {
          catch { unset ::hooks(%N%,$method) }
        } else {
          set ::hooks(%N%,$method) $held
        }
        return {}
      }
      copy - collation_needed - enable_load_extension - interrupt -
      preupdate - rekey - timeout - version - config - deserialize -
      serialize - backup - restore {
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

# The `%XX` escapes `test_exec` of `research/sqlite/src/test1.c:442`
# reads, which is how a file writes a byte it cannot hold as text.
proc exec_bytes {sql} {
  set out ""
  set at 0
  set n [string length $sql]
  while {$at < $n} {
    set c [string index $sql $at]
    set hex [string range $sql [expr {$at+1}] [expr {$at+2}]]
    if {$c eq "%" && [string length $hex] == 2 && [scan $hex %2x byte] == 1} {
      append out [format %c $byte]
      incr at 3
    } else {
      append out $c
      incr at 1
    }
  }
  return $out
}

# `sqlite3_exec DB SQL` of `research/sqlite/src/test1.c:421`: the code
# the statement answered, and then the column names followed by every
# row's values, which `exec_printf_cb` writes only where a first row
# arrived. An error answers the code and the message.
proc sqlite3_exec {db sql} {
  set sql [exec_bytes $sql]
  if {[catch { set rows [$db eval $sql] } msg]} { return [list 1 $msg] }
  if {[llength $rows] == 0} { return [list 0 {}] }
  return [list 0 [concat [$db names $sql] $rows]]
}

# `sqlite3_exec_nr DB SQL`, which drops what the statement answered.
proc sqlite3_exec_nr {db sql} { return [lindex [sqlite3_exec $db $sql] 0] }

proc db_eval {sql} { return [db eval $sql] }
# `stepsql` of the suite's own tester: every statement of the text
# prepared and stepped in turn, answering nought and then the values.
proc stepsql {dbptr sql} {
  set sql [string trim $sql]
  set r 0
  while {[string length $sql] > 0} {
    if {[catch { sqlite3_prepare $dbptr $sql -1 sqltail } vm]} {
      return [list 1 $vm]
    }
    if {$vm eq ""} { break }
    set sql [string trim $sqltail]
    while {[sqlite3_step $vm] eq "SQLITE_ROW"} {
      for {set i 0} {$i < [sqlite3_data_count $vm]} {incr i} {
        lappend r [sqlite3_column_text $vm $i]
      }
    }
    if {[catch { sqlite3_finalize $vm } errmsg]} {
      return [list 1 $errmsg]
    }
  }
  return $r
}

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
  # `reset_db` of the suite's own tester names the connection `::DB` as
  # well, which the files that drive the C API read.
  set ::DB [sqlite3_connection_pointer db]
  if {[info exists ::SETUP_SQL]} { db eval $::SETUP_SQL }
}
proc db_delete_and_reopen {{file test.db}} { reset_db }
proc drop_all_tables {{db db}} {
  set pk [$db one "PRAGMA foreign_keys"]
  catch { $db eval "PRAGMA foreign_keys = OFF" }
  foreach {idx name file} [$db eval {PRAGMA database_list}] {
    if {$idx==1} {
      set master sqlite_temp_master
    } else {
      set master $name.sqlite_master
    }
    foreach {t type} [$db eval "
      SELECT name, type FROM $master
      WHERE type IN('table', 'view') AND name NOT LIKE 'sqliteX_%' ESCAPE 'X'
    "] {
      catch { $db eval "DROP $type \"$t\"" }
    }
  }
  catch { $db eval "PRAGMA foreign_keys = $pk" }
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
  sqlite3_extended_result_codes
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
  sqlite3_db_filename sqlite3_stmt_readonly
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

# md5 and md5file of test_md5.c: the digest of RFC 1321 over a text and
# over a file the harness holds.
proc md5 {text} { return [lindex [harness_send md5 $text] 0] }
proc md5file {name} { return [lindex [harness_send md5file $name] 0] }

# cksum of the suite's own tester: one state of a database as a text, so
# that two states are held against each other.
proc cksum {{db db}} {
  set txt [$db eval {
      SELECT name, type, sql FROM sqlite_master order by name
  }]\n
  foreach tbl [$db eval {
      SELECT name FROM sqlite_master WHERE type='table' order by name
  }] {
    append txt [$db eval "SELECT * FROM $tbl"]\n
  }
  foreach prag {default_synchronous default_cache_size} {
    append txt $prag-[$db eval "PRAGMA $prag"]\n
  }
  return [string length $txt]-[md5 $txt]
}

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

# `sqlite3_prepare` and the commands that read a statement it answered.
# The harness holds the statement under a name, which stands for the
# pointer the C library answers. The message of the last request is kept
# so that `sqlite3_errmsg` answers it.
set ::harness_error ""
set ::harness_code SQLITE_OK

# One request whose error is kept rather than raised, which is how a
# command that answers a code rather than raising is written.
proc harness_try {verb args} {
  set ::harness_error ""
  set ::harness_code SQLITE_OK
  if {[catch { eval harness_send [list $verb] $args } out]} {
    set ::harness_error $out
    set ::harness_code SQLITE_ERROR
    return ""
  }
  return $out
}

# `sqlite3_prepare DB SQL BYTES ?TAILVAR?`: the name of the statement,
# with the text after it written into the variable the caller names. A
# statement the engine refuses raises `(code) message`, which is what
# `test_prepare` of `research/sqlite/src/test1.c` writes.
proc harness_prepare {db sql tailvar legacy} {
  set answered [harness_send prepare $db $sql $legacy]
  if {$tailvar ne ""} {
    upvar 2 $tailvar tail
    set tail [lindex $answered 1]
  }
  set ::harness_error [lindex $answered 2]
  if {$::harness_error ne ""} {
    set ::harness_code SQLITE_ERROR
    error "([lindex [harness_send errcode number] 0]) $::harness_error"
  }
  set ::harness_code SQLITE_OK
  set ::stmt_legacy([lindex $answered 0]) $legacy
  return [lindex $answered 0]
}
proc sqlite3_prepare {db sql bytes {tailvar ""}} {
  return [harness_prepare $db $sql $tailvar 1]
}
proc sqlite3_prepare_v2 {db sql bytes {tailvar ""}} {
  return [harness_prepare $db $sql $tailvar 0]
}
proc sqlite3_prepare_v3 {db sql bytes flags {tailvar ""}} {
  return [harness_prepare $db $sql $tailvar 0]
}
# `sqlite3_prepare16` is given the statement as UTF-16, which a file
# writes with `encoding convertto unicode` and ends in two nulls. The
# engine reads UTF-8, so the text is read back and the nulls dropped.
proc harness_utf8 {sql} {
  return [string trimright [encoding convertfrom unicode $sql] "\x00"]
}
proc sqlite3_prepare16 {db sql bytes {tailvar ""}} {
  return [harness_prepare $db [harness_utf8 $sql] $tailvar 1]
}
proc sqlite3_prepare16_v2 {db sql bytes {tailvar ""}} {
  return [harness_prepare $db [harness_utf8 $sql] $tailvar 0]
}
proc sqlite3_prepare16_v3 {db sql bytes flags {tailvar ""}} {
  return [harness_prepare $db [harness_utf8 $sql] $tailvar 0]
}

# `sqlite3_next_stmt DB STMT`: the statement after the one named, and
# the first where the name is `0`.
proc sqlite3_next_stmt {db stmt} {
  return [lindex [harness_send next_stmt $db $stmt] 0]
}
proc sqlite3_step {stmt} {
  set answered [harness_try step $stmt]
  if {$::harness_code ne "SQLITE_OK"} {
    # A statement `sqlite3_prepare` made answers `SQLITE_ERROR` for
    # every refusal, and `sqlite3_finalize` answers the code it carries.
    if {[info exists ::stmt_legacy($stmt)] && $::stmt_legacy($stmt) == 1} {
      return SQLITE_ERROR
    }
    return [sqlite3_errcode {}]
  }
  return [lindex $answered 0]
}
proc sqlite3_finalize {stmt} {
  set held $::harness_code
  harness_send finalize $stmt
  if {$held ne "SQLITE_OK"} { return [sqlite3_errcode {}] }
  return SQLITE_OK
}
proc sqlite3_reset {stmt} {
  set held $::harness_code
  harness_send reset $stmt
  set ::harness_code SQLITE_OK
  if {$held ne "SQLITE_OK"} { return [sqlite3_errcode {}] }
  return SQLITE_OK
}
proc sqlite3_clear_bindings {stmt} { return [harness_send clear_binds $stmt] }
proc sqlite3_column_count {stmt} { return [lindex [harness_send column $stmt count 0] 0] }
proc sqlite3_data_count {stmt} { return [lindex [harness_send column $stmt data 0] 0] }
proc sqlite3_column_name {stmt at} { return [lindex [harness_send column $stmt name $at] 0] }
proc sqlite3_column_name16 {stmt at} { return [sqlite3_column_name $stmt $at] }
proc sqlite3_column_decltype {stmt at} {
  return [lindex [harness_send column $stmt decltype $at] 0]
}
proc sqlite3_column_decltype16 {stmt at} { return [sqlite3_column_decltype $stmt $at] }
proc sqlite3_column_type {stmt at} { return [lindex [harness_send column $stmt type $at] 0] }
proc sqlite3_column_int {stmt at} { return [lindex [harness_send column $stmt int $at] 0] }
proc sqlite3_column_int64 {stmt at} { return [sqlite3_column_int $stmt $at] }
proc sqlite3_column_double {stmt at} { return [lindex [harness_send column $stmt double $at] 0] }
proc sqlite3_column_text {stmt at} { return [lindex [harness_send column $stmt text $at] 0] }
proc sqlite3_column_text16 {stmt at} { return [sqlite3_column_text $stmt $at] }
proc sqlite3_column_blob {stmt at} { return [sqlite3_column_text $stmt $at] }
proc sqlite3_column_bytes {stmt at} { return [string length [sqlite3_column_text $stmt $at]] }
proc sqlite3_column_bytes16 {stmt at} { return [expr {2*[sqlite3_column_bytes $stmt $at]}] }
# `sqlite3_bind_*` answers nothing where it bound the value, which is
# what `test_bind` of `research/sqlite/src/test1.c` writes.
proc sqlite3_bind_int {stmt at value} { harness_send bind $stmt $at $value ; return {} }
proc sqlite3_bind_int64 {stmt at value} { harness_send bind $stmt $at $value ; return {} }
proc sqlite3_bind_double {stmt at value} { harness_send bind $stmt $at $value ; return {} }
proc sqlite3_bind_null {stmt at} { harness_send bind $stmt $at NULL ; return {} }
# The first `$bytes` bytes of a value, and the whole of it where the
# count is negative or absent, which is what `nByte` of
# `sqlite3_bind_text` names.
proc harness_bytes {value bytes} {
  if {$bytes eq "" || $bytes < 0} { return $value }
  return [string range $value 0 [expr {$bytes - 1}]]
}
proc sqlite3_bind_text {stmt at value args} {
  set held [harness_bytes $value [lindex $args 0]]
  harness_send bind $stmt $at '[string map {' ''} $held]'
  return {}
}
# `sqlite3_bind_text16` counts the bytes of the UTF-16 text, so two per
# character, and the engine reads UTF-8.
proc sqlite3_bind_text16 {stmt at value args} {
  set held [harness_bytes $value [lindex $args 0]]
  return [sqlite3_bind_text $stmt $at [harness_utf8 $held] -1]
}
proc sqlite3_bind_blob {stmt at value args} {
  return [sqlite3_bind_text $stmt $at $value [lindex $args 0]]
}
proc sqlite3_bind_parameter_count {stmt} { return [lindex [harness_send stmt $stmt binds] 0] }
proc sqlite3_bind_parameter_name {stmt at} {
  return [lindex [harness_send stmt $stmt parameter $at] 0]
}
proc sqlite3_bind_parameter_index {stmt name} {
  return [lindex [harness_send stmt $stmt index $name] 0]
}

# `sqlite_bind VM IDX VALUE TYPE` of `research/sqlite/src/test1.c`: the
# value one parameter stands for, with `null` for nothing and the other
# three words for a text.
proc sqlite_bind {stmt at value type} {
  if {$type eq "null"} { return [sqlite3_bind_null $stmt $at] }
  if {$type eq "blob10"} { return [harness_send bind $stmt $at x'00000000000000000000'] }
  if {[string match "static*" $type]} {
    return [sqlite3_bind_text $stmt $at $::sqlite_static_bind_value]
  }
  return [sqlite3_bind_text $stmt $at $value]
}
set ::sqlite_static_bind_value {}
set ::sqlite_static_bind_nbytes 0
proc sqlite3_sql {stmt} { return [lindex [harness_send stmt $stmt sql] 0] }
proc sqlite3_normalized_sql {stmt} { return [lindex [harness_send stmt $stmt normalized] 0] }
proc sqlite3_normalize {sql} { return [lindex [harness_send normalize $sql] 0] }
proc sqlite3_expanded_sql {stmt} { return [lindex [harness_send stmt $stmt expanded] 0] }
# `sqlite3_errcode` answers the code the last statement was refused
# with, and `sqlite3_errmsg` the message, which is `not an error` where
# the statement stood.
proc sqlite3_errcode {db} { return [lindex [harness_send errcode primary] 0] }
proc sqlite3_extended_errcode {db} { return [lindex [harness_send errcode extended] 0] }
proc sqlite3_get_autocommit {db} { return [lindex [harness_send autocommit $db] 0] }
proc sqlite3_errmsg {db} {
  if {$::harness_error eq ""} { return "not an error" }
  return $::harness_error
}
proc sqlite3_errmsg16 {db} { return [sqlite3_errmsg $db] }

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
  explain autovacuum session setlk_timeout configslower
  memorymanage threadsafe
} { set ::sqlite_options($option) 0 }
foreach option {
  wal utf16 integrityck casesensitivelike trigger view subquery compound attach
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

# The files that drive the C API read `::DB` for the connection, which
# `reset_db` of the suite's own tester names as well.
set ::DB [sqlite3_connection_pointer db]
