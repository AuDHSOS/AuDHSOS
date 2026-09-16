# SPDX-License-Identifier: AGPL-3.0-only
# Copyright (C) 2026 Manuel Baesler and contributors
# Fetch the Adobe technical notes the OpenType specification defers to,
# byte for byte. Run from anywhere; writes beside this script. See README.md.
set -eu

dir=$(cd "$(dirname "$0")" && pwd)
base=https://adobe-type-tools.github.io/font-tech-notes/pdfs

for f in 5176.CFF.pdf 5177.Type2.pdf 5902.AdobePSNameGeneration.pdf; do
	curl -sSL --fail --max-time 300 "$base/$f" -o "$dir/$f"
	printf '%s  %s\n' "$(shasum -a 256 "$dir/$f" | cut -d' ' -f1)" "$f"
done
