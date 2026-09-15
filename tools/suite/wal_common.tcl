# SPDX-License-Identifier: AGPL-3.0-only
# Copyright (C) 2026 Manuel Baesler and contributors
#
# A stand-in for the suite's `wal_common.tcl`: every command in it
# reads the bytes of a write-ahead log through the C library's own
# virtual file system.

proc wal_frame_count {args} { error "this harness has no wal_frame_count" }
proc wal_file_size {args} { error "this harness has no wal_file_size" }
proc wal_cksum {args} { error "this harness has no wal_cksum" }
proc log_file_size {args} { error "this harness has no log_file_size" }
proc set_tvfs_hdr {args} { error "this harness has no set_tvfs_hdr" }
proc incr_tvfs_hdr {args} { error "this harness has no incr_tvfs_hdr" }
proc wal_fix_walindex_cksum {args} { error "this harness has no wal_fix_walindex_cksum" }
proc wal_check_journal_mode {args} {}
proc wal_set_journal_mode {args} {}
proc do_wal_checkpoint {args} { return {0 -1 -1} }
