# SPDX-License-Identifier: AGPL-3.0-only
# Copyright (C) 2026 Manuel Baesler and contributors

cd "$(dirname "$0")/.."
export PATH="$HOME/.cargo/bin:$PATH"

PAGER=cat CARGO_TERM_COLOR=never ~/.cargo/bin/cargo xtask "$@" 2>&1 | tail -60
