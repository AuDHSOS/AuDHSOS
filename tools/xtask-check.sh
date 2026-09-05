# SPDX-License-Identifier: AGPL-3.0-only
# Copyright (C) 2026 Manuel Baesler and contributors

# `sh tools/xtask.sh check`, as its own entry point. Start it as
# `sh tools/xtask-check.sh`; the SPDX header must be the first line.

exec sh "$(dirname "$0")/xtask.sh" check "$@"
