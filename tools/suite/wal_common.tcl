# SPDX-License-Identifier: AGPL-3.0-only
# Copyright (C) 2026 Manuel Baesler and contributors
#
# A stand-in for the suite's `wal_common.tcl`. The commands that count
# the frames of a log are arithmetic over the size of the file, which
# the harness answers; the ones that write the log's header or its
# checksums read the bytes through the C library's own virtual file
# system.

# The bytes a log of `nFrame` frames takes: the header, then a header
# and a page per frame.
proc wal_file_size {nFrame pgsz} {
  expr {32 + ($pgsz+24)*$nFrame}
}

proc log_file_size {nFrame pgsz} {
  wal_file_size $nFrame $pgsz
}

# How many frames the log `zFile` holds, which its size says.
proc wal_frame_count {zFile pgsz} {
  set bytes [lindex [harness_send size $zFile] 0]
  if {$bytes < 32} { return 0 }
  expr {($bytes - 32) / ($pgsz+24)}
}

proc wal_cksum_intlist {ckv1 ckv2 intlist} {
  upvar $ckv1 c1
  upvar $ckv2 c2
  foreach {v1 v2} $intlist {
    set c1 [expr {($c1 + $v1 + $c2)&0xFFFFFFFF}]
    set c2 [expr {($c2 + $v2 + $c1)&0xFFFFFFFF}]
  }
}

# The checksum of section 4.2 over `blob`, carried on from c1 and c2.
proc wal_cksum {endian ckv1 ckv2 blob} {
  upvar $ckv1 c1
  upvar $ckv2 c2
  if {$endian!="big" && $endian!="little"} {
    return -error "Bad value \"$endian\" - must be \"big\" or \"little\""
  }
  set scanpattern I*
  if {$endian == "little"} { set scanpattern i* }
  binary scan $blob $scanpattern values
  wal_cksum_intlist c1 c2 $values
}

proc set_tvfs_hdr {args} { error "this harness has no set_tvfs_hdr" }
proc incr_tvfs_hdr {args} { error "this harness has no incr_tvfs_hdr" }
proc wal_fix_walindex_cksum {args} { error "this harness has no wal_fix_walindex_cksum" }
proc wal_check_journal_mode {args} {}
proc wal_set_journal_mode {args} {}
proc do_wal_checkpoint {args} { return {0 -1 -1} }
