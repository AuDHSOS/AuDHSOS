# SPDX-License-Identifier: AGPL-3.0-only
# Copyright (C) 2026 Manuel Baesler and contributors

# The entry point for the xtask on a machine whose PATH carries a foreign
# rustc ahead of rustup's. Runs without a shebang, so start it as
# `sh tools/xtask.sh <subcommand>`; the SPDX header must be the first line.

cd "$(dirname "$0")/.." || exit 1
PATH="$HOME/.cargo/bin:$PATH"
export PATH
PAGER=cat
CARGO_TERM_COLOR=never
export PAGER CARGO_TERM_COLOR

exec "$HOME/.cargo/bin/cargo" xtask "$@"
