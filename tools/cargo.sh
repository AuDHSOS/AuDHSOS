# SPDX-License-Identifier: AGPL-3.0-only
# Copyright (C) 2026 Manuel Baesler and contributors

# An ordinary Cargo command on a machine whose PATH carries a foreign
# rustc ahead of rustup's, for what is not an xtask subcommand:
# `sh tools/cargo.sh fmt -p <crate>`. Runs without a shebang, so start it
# with `sh`; the SPDX header must be the first line.

cd "$(dirname "$0")/.." || exit 1
PATH="$HOME/.cargo/bin:$PATH"
export PATH
PAGER=cat
CARGO_TERM_COLOR=never
export PAGER CARGO_TERM_COLOR

exec "$HOME/.cargo/bin/cargo" "$@"
