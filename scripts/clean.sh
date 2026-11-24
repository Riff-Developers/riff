#!/usr/bin/env bash
set -euo pipefail

# Reclaim disk space used by Riff build artifacts and debug leftovers.
# Usage:
#   ./scripts/clean.sh [options]
#
# Options:
#   --all         Remove every build directory entirely (target/, BUILD/, builddir/,
#                 build/, .flatpak-builder/). Requires re-running meson setup.
#   --app-cache   Also remove the app's runtime caches (~/.cache/riff*)
#   -n, --dry-run Show what would be removed without deleting anything
#   -y, --yes     Don't ask for confirmation
#   -h, --help    Show this help

REPO_ROOT="$(cd "$(dirname "$0")/.." && pwd)"

# Build directories: target (build.sh + cargo), BUILD (Justfile), builddir/build (legacy)
BUILD_DIRS=(target BUILD builddir build)
LEGACY_DIRS=(builddir build)

ALL=0
APP_CACHE=0
DRY_RUN=0
YES=0

usage() {
    sed -n '3,/^$/s/^# \?//p' "$0"
    exit 0
}

die() {
    echo "error: $*" >&2
    exit 1
}

while [[ $# -gt 0 ]]; do
    case "$1" in
        --all) ALL=1 ;;
        --app-cache) APP_CACHE=1 ;;
        -n|--dry-run) DRY_RUN=1 ;;
        -y|--yes) YES=1 ;;
        -h|--help) usage ;;
        *) die "unknown option: $1 (see --help)" ;;
    esac
    shift
done

cd "$REPO_ROOT"

TARGETS=()

add() {
    local path
    for path in "$@"; do
        [[ -e "$path" ]] && TARGETS+=("$path")
    done
    return 0
}

# Cargo profile directories live directly in target/ (plain cargo) and in
# <builddir>/src/ (meson's cargo target dir).
profile_dirs() {
    local dir
    for dir in "${BUILD_DIRS[@]}"; do
        [[ -d "$dir" ]] || continue
        find "$dir" "$dir/src" -mindepth 1 -maxdepth 1 -type d \
            \( -name debug -o -name release \) 2>/dev/null
    done
}

if [[ $ALL -eq 1 ]]; then
    add "${BUILD_DIRS[@]}" .flatpak-builder
else
    add "${LEGACY_DIRS[@]}"

    while IFS= read -r profile; do
        if [[ "$(basename "$profile")" == debug ]]; then
            add "$profile"
        else
            add "$profile/incremental" "$profile/examples"
        fi
    done < <(profile_dirs)
fi

while IFS= read -r -d '' cache; do
    add "$cache"
done < <(find . \( -path ./.git -o -path ./scripts/.venv \) -prune -o -type d -name __pycache__ -printf '%P\0')

if [[ $APP_CACHE -eq 1 ]]; then
    add "${XDG_CACHE_HOME:-$HOME/.cache}"/riff*
fi

# Drop entries nested inside another entry so sizes aren't double counted.
mapfile -t TARGETS < <(printf '%s\n' "${TARGETS[@]}" | sort -u | awk '
    { for (i = 1; i <= n; i++) if (index($0, kept[i] "/") == 1) next; kept[++n] = $0; print }
')

if [[ ${#TARGETS[@]} -eq 0 || -z "${TARGETS[0]}" ]]; then
    echo "Nothing to clean."
    exit 0
fi

# Skip empty entries (e.g. examples/ dirs that were never populated).
TOTAL=0
NONEMPTY=()
for path in "${TARGETS[@]}"; do
    size=$(du -sb "$path" 2>/dev/null | cut -f1)
    [[ -n "$(find "$path" -mindepth 1 -print -quit 2>/dev/null)" ]] || continue
    NONEMPTY+=("$path")
    TOTAL=$((TOTAL + size))
    printf '%10s  %s\n' "$(numfmt --to=iec "$size")" "$path"
done
TARGETS=("${NONEMPTY[@]}")

if [[ ${#TARGETS[@]} -eq 0 ]]; then
    echo "Nothing to clean."
    exit 0
fi

echo "----------"
printf '%10s  total\n' "$(numfmt --to=iec "$TOTAL")"

if [[ $DRY_RUN -eq 1 ]]; then
    echo "Dry run: nothing removed."
    exit 0
fi

if [[ $YES -eq 0 ]]; then
    read -r -p "Remove these? [y/N] " answer
    [[ "$answer" =~ ^[Yy]$ ]] || { echo "Aborted."; exit 1; }
fi

for path in "${TARGETS[@]}"; do
    rm -rf -- "$path"
done
echo "Freed $(numfmt --to=iec "$TOTAL")."
