# SPDX-License-Identifier: AGPL-3.0-only
# Copyright (C) 2026 Manuel Baesler and contributors
#
# A stand-in for the suite's `fts3_common.tcl`: full-text search is a
# virtual table module this engine does not have.

proc fts3_build_db_1 {args} { error "this harness has no fts3" }
proc fts3_build_db_2 {args} { error "this harness has no fts3" }
proc fts3_integrity_check {args} { error "this harness has no fts3" }
proc check_terms {args} { error "this harness has no fts3" }
proc check_doclist {args} { error "this harness has no fts3" }
proc fts3_zero_long_segments {args} { return 0 }
