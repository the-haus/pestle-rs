#!/bin/sh
# Takes the Public Suffix List and its tests as publicsuffix.org serves them
# (pestle_html::psl, registrableDomain). Commit both with the list's VERSION.
set -eu
here=$(cd "$(dirname "$0")/.." && pwd)
curl -fsSL -o "$here/crates/pestle-html/data/public_suffix_list.dat" https://publicsuffix.org/list/public_suffix_list.dat
curl -fsSL -o "$here/crates/pestle-html/tests/data/test_psl.txt" https://raw.githubusercontent.com/publicsuffix/list/main/tests/test_psl.txt
grep '^// VERSION:' "$here/crates/pestle-html/data/public_suffix_list.dat"
