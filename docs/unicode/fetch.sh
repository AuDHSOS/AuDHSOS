# SPDX-License-Identifier: AGPL-3.0-only
# Copyright (C) 2026 Manuel Baesler and contributors
# Fetch the annexes, reports, character database and conformance test files of
# Unicode 18.0.0, byte for byte, plus the figure files the reports name.
# Run from anywhere; writes beside this script. See README.md.
set -eu

dir=$(cd "$(dirname "$0")" && pwd)
base=https://www.unicode.org
version=18.0.0

reports='tr9:52 tr11:46 tr14:57 tr29:49 tr37:16 tr50:35 tr51:31'
ucd='Scripts.txt ScriptExtensions.txt DerivedCoreProperties.txt
extracted/DerivedBidiClass.txt BidiBrackets.txt BidiMirroring.txt
ArabicShaping.txt PropertyValueAliases.txt UnicodeData.txt EastAsianWidth.txt LineBreak.txt BidiTest.txt
BidiCharacterTest.txt auxiliary/GraphemeBreakProperty.txt
auxiliary/WordBreakProperty.txt auxiliary/GraphemeBreakTest.txt
auxiliary/WordBreakTest.txt auxiliary/LineBreakTest.txt emoji/emoji-data.txt'

sum() { printf '%s  %s\n' "$(shasum -a 256 "$dir/$1" | cut -d' ' -f1)" "$1"; }

for r in $reports; do
	n=${r%%:*}
	v=${r##*:}
	mkdir -p "$dir/reports/$n"
	curl -sSL --fail --max-time 120 "$base/reports/$n/$n-$v.html" \
		-o "$dir/reports/$n/$n-$v.html"
	sum "reports/$n/$n-$v.html"
done

for f in $ucd; do
	mkdir -p "$dir/ucd/$(dirname "$f")"
	curl -sSL --fail --max-time 300 "$base/Public/$version/ucd/$f" \
		-o "$dir/ucd/$f"
	sum "ucd/$f"
done

# The figure files. A report names them by a relative path, except UAX #11,
# which names its two by the address they have on the server; both forms are
# fetched into the path below the report's own directory.
for r in $reports; do
	n=${r%%:*}
	v=${r##*:}
	grep -oE 'src="[^"]*"' "$dir/reports/$n/$n-$v.html" |
		sed 's/src="//;s/"$//' |
		sed "s|^$base/reports/$n/||" |
		grep -v '^data:' | grep -v '^/' | grep -v '^https\{0,1\}://' |
		LC_ALL=C sort -u |
		while read -r img; do
			mkdir -p "$dir/reports/$n/$(dirname "$img")"
			curl -sSL --fail --max-time 120 \
				"$base/reports/$n/$img" -o "$dir/reports/$n/$img"
			printf '%s  %s\n' \
				"$(shasum -a 256 "$dir/reports/$n/$img" | cut -d' ' -f1)" \
				"reports/$n/$img"
		done
done
