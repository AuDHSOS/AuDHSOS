# SPDX-License-Identifier: AGPL-3.0-only
# Copyright (C) 2026 Manuel Baesler and contributors
#
# What `tclsh` is started with: open the line to the harness, read the
# tester, and read one file of the suite. The arguments are the file to
# read and the port the harness listens on.

set ::testdir [file dirname [info script]]
set ::harness_port [lindex $argv 1]
set ::harness [socket 127.0.0.1 $::harness_port]
fconfigure $::harness -translation binary -encoding binary -buffering full

source $::testdir/tester.tcl

# A file of the suite reads its helper files by
# `set testdir [file dirname $argv0]`, so `argv0` names the runner and
# the helpers a file reads are this repository's own.
set ::argv0 [info script]
if {[catch { uplevel #0 [list source [lindex $argv 0]] } msg]} {
  set msg "$msg\n$::errorInfo"
  harness_send stopped $msg
}
finish_test
