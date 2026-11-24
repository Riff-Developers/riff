#!/usr/bin/env bash
set -euo pipefail

# Checks that po/POTFILES and po/riff.pot are up to date.

REPO_ROOT="$(cd "$(dirname "$0")/../.." && pwd)"
cd "$REPO_ROOT"

TMP_DIR="$(mktemp -d)"
trap 'rm -rf "$TMP_DIR"' EXIT
FAILED=0

grep -vE '^(#|\s*$)' po/POTFILES | sort > "$TMP_DIR/listed"

echo "=== POTFILES ==="
MISSING=$(while read -r f; do [[ -e "$f" ]] || echo "$f"; done < "$TMP_DIR/listed")
UNLISTED=$(grep -rlE '\bn?gettext\(|(^|[^[:alnum:]_])_\("' src crates --include=*.rs --include=*.blp \
    | sort | comm -23 - "$TMP_DIR/listed")
if [[ -n "$MISSING" ]]; then
    echo "Listed in po/POTFILES but missing:"
    while read -r f; do echo "  $f"; done <<< "$MISSING"
    FAILED=1
fi
if [[ -n "$UNLISTED" ]]; then
    echo "Contain translatable strings but not listed in po/POTFILES:"
    while read -r f; do echo "  $f"; done <<< "$UNLISTED"
    FAILED=1
fi
if [[ "$FAILED" -ne 0 ]]; then
    echo "FAIL"
    exit 1
fi
echo "PASS"

echo ""
echo "=== riff.pot ==="
xgettext --package-name=riff --from-code=UTF-8 --add-comments \
    --keyword=_ --keyword=N_ --keyword=C_:1c,2 --keyword=NC_:1c,2 \
    --keyword=g_dcgettext:2 --keyword=g_dngettext:2,3 --keyword=g_dpgettext2:2c,3 \
    --files-from="$TMP_DIR/listed" --output="$TMP_DIR/riff.pot" 2>/dev/null
if msgcmp --use-untranslated "$TMP_DIR/riff.pot" po/riff.pot \
    && msgcmp --use-untranslated po/riff.pot "$TMP_DIR/riff.pot"; then
    echo "PASS"
else
    echo "FAIL — po/riff.pot is out of date."
    FAILED=1
fi



echo ""
if [[ "$FAILED" -ne 0 ]]; then
    echo "Translation checks failed. Run './scripts/build.sh pot'."
    exit 1
fi
echo "All translation checks passed."
