# SPDX-License-Identifier: AGPL-3.0-only
# Copyright (C) 2026 Manuel Baesler and contributors
# Fetch the OpenType specification as Microsoft serves it: one file per page,
# byte for byte, plus the figure files the pages name by a relative path.
# Run from anywhere; writes beside this script. See README.md.
set -eu

dir=$(cd "$(dirname "$0")" && pwd)
base=https://learn.microsoft.com/en-us/typography/opentype/spec

pages='index overview ttochap1 otvaroverview otff chapter2 otvarcommonformats
avar base cbdt cblc cff cff2 cmap colr cpal cvar cvt dsig ebdt eblc ebsc fpgm
fvar gasp gdef glyf gpos gsub gvar hdmx head hhea hmtx hvar jstf kern loca ltsh
math maxp merg meta mvar name namesmp os2 ibmfc pclt post prep sbix stat svg
vdmx vhea vmtx vorg vvar ttoreg scripttags languagetags featuretags features_ae
features_fj features_ko features_pt features_uz baselinetags dvaraxisreg
dvaraxistag_ital dvaraxistag_opsz dvaraxistag_slnt dvaraxistag_wdth
dvaraxistag_wght errata recom ttch01 tt_instructing_glyphs tt_instructions
tt_graphics_state ompl glyphformatcomparison changes'

for p in $pages; do
	case $p in
	index) url=$base/ ;;
	*) url=$base/$p ;;
	esac
	curl -sSL --fail --max-time 120 "$url" -o "$dir/$p.html"
	printf '%s  %s\n' "$(shasum -a 256 "$dir/$p.html" | cut -d' ' -f1)" "$p.html"
done

# The figure files, at the relative paths the pages name them by.
for p in $pages; do
	grep -oE '<img[^>]*src="[^"]*"' "$dir/$p.html" |
		grep -oE 'src="[^"]*"' | sed 's/src="//;s/"$//' |
		grep -v '^data:' | grep -v '^https\{0,1\}://' || true
done | sort -u | while read -r img; do
	mkdir -p "$dir/$(dirname "$img")"
	curl -sSL --fail --max-time 120 "$base/$img" -o "$dir/$img"
	printf '%s  %s\n' "$(shasum -a 256 "$dir/$img" | cut -d' ' -f1)" "$img"
done
