#!/usr/bin/env bash
set -euo pipefail

# Build script for Riff.
# Usage:
#   ./scripts/build.sh [command] [options]
#
# Commands:
#   dev       Configure and build a debug build (default)
#   release   Configure and build an optimized release build
#   test      Run the test suite
#   pot       Regenerate translation pot files
#   update-po Update .po files from the pot
#   cargo-src Regenerate flatpak cargo sources
#   clean     Remove the build directory
#
# Options:
#   --prefix PATH     Install prefix (default: $HOME/.local)
#   --builddir DIR    Build directory name (default: target)
#   --offline         Build in offline mode (no network fetches)
#   --reconfigure     Force reconfigure even if build dir exists
#   --features LIST   Comma-separated list of features to enable
#   --install         Install after building (dev/release only)
#   --fresh-install   Install as if for the first time (dev/release only):
#                     once the build succeeds, removes everything Riff keeps
#                     for this user (data, caches, Soloist and its pairing,
#                     keyring items, settings) before installing. Doesn't ask.

REPO_ROOT="$(cd "$(dirname "$0")/.." && pwd)"

# Defaults
BUILD_DIR="target"
PREFIX="$HOME/.local"
OFFLINE="false"
RECONFIGURE=0
FEATURES=""
INSTALL=0
FRESH=0

usage() {
    sed -n '3,/^$/s/^# \?//p' "$0"
    exit 0
}

die() {
    echo "Error: $*" >&2
    exit 1
}

# Parse command
COMMAND="${1:-dev}"
case "$COMMAND" in
    dev|release|test|pot|cargo-src|clean|help)
        shift || true
        ;;
    -h|--help)
        shift || true
        ;;
    --*)
        # First arg is an option, not a command — use default
        COMMAND="dev"
        ;;
    *)
        shift || true
        ;;
esac

# Parse options
while [[ $# -gt 0 ]]; do
    case "$1" in
        --prefix)
            PREFIX="${2:?--prefix requires a path}"
            shift 2
            ;;
        --builddir)
            BUILD_DIR="${2:?--builddir requires a name}"
            shift 2
            ;;
        --offline)
            OFFLINE="true"
            shift
            ;;
        --reconfigure)
            RECONFIGURE=1
            shift
            ;;
        --features)
            FEATURES="${2:?--features requires a value}"
            shift 2
            ;;
        --install)
            INSTALL=1
            shift
            ;;
        --fresh-install)
            FRESH=1
            INSTALL=1
            shift
            ;;
        -h|--help)
            usage
            ;;
        *)
            die "Unknown option: $1"
            ;;
    esac
done

BUILD_PATH="$REPO_ROOT/$BUILD_DIR"

# Validate --install usage
if [[ "$INSTALL" -eq 1 && "$COMMAND" != "dev" && "$COMMAND" != "release" ]]; then
    die "--install and --fresh-install are only valid with 'dev' or 'release' commands"
fi

DATA_HOME="${XDG_DATA_HOME:-$HOME/.local/share}"
CACHE_HOME="${XDG_CACHE_HOME:-$HOME/.cache}"
FRESH_PATHS=(
    "$DATA_HOME/riff/soloist"
    "$DATA_HOME/riff/now_playing_textures"
    "$CACHE_HOME/riff"
)
FRESH_KEYRING_ATTRS=("riff" "riff-session" "riff-player-key")
SETTINGS_PATH="/dev/diegovsky/Riff/"

wipe_user_data() {
    echo "==> Removing Riff's user data..."
    local path attr var
    for path in "${FRESH_PATHS[@]}"; do
        if [[ -e "$path" ]]; then
            rm -rf -- "$path"
            echo "  removed $path"
        fi
    done
    if command -v secret-tool >/dev/null 2>&1; then
        for attr in "${FRESH_KEYRING_ATTRS[@]}"; do
            secret-tool clear "$attr" yes 2>/dev/null || true
        done
        echo "  cleared keyring items"
    else
        echo "Warning: secret-tool isn't installed; keyring items were left." >&2
    fi
    if command -v dconf >/dev/null 2>&1; then
        dconf reset -f "$SETTINGS_PATH"
        echo "  reset settings"
    else
        echo "Warning: dconf isn't installed; settings were left." >&2
    fi
    for var in RIFF_SOLOIST_API_KEY RIFF_SOLOIST_BIN; do
        if [[ -n "${!var:-}" ]]; then
            echo "Warning: $var is set; Riff will use it, so setup won't be fully fresh." >&2
        fi
    done
}

setup_build() {
    local buildtype="$1"
    local setup_args=(
        "$BUILD_PATH"
        "-Dbuildtype=$buildtype"
        "-Doffline=$OFFLINE"
        "--prefix=$PREFIX"
    )

    if [[ -n "$FEATURES" ]]; then
        setup_args+=("-Dfeatures=$FEATURES")
    fi

    if [[ -d "$BUILD_PATH" ]]; then
        if [[ "$RECONFIGURE" -eq 1 ]]; then
            setup_args+=("--reconfigure")
        else
            setup_args+=("--reconfigure")
        fi
    fi

    echo "==> Configuring ($buildtype)..."
    meson setup "${setup_args[@]}"
}

do_build() {
    echo "==> Building..."
    ninja -C "$BUILD_PATH"
}

do_install() {
    if [[ "$FRESH" -eq 1 ]]; then
        wipe_user_data
    fi
    echo "==> Installing to $PREFIX..."
    ninja install -C "$BUILD_PATH"
}

case "$COMMAND" in
    dev)
        export RUST_LOG="${RUST_LOG:-riff=debug,librespot=error}"
        setup_build "debug"
        do_build
        if [[ "$INSTALL" -eq 1 ]]; then
            do_install
            echo ""
            echo "Installed. Run with:"
            echo "  RUST_LOG='$RUST_LOG' $PREFIX/bin/riff"
        else
            echo ""
            echo "Debug build complete. Install with:"
            echo "  $0 dev --install"
            echo "Or run directly:"
            echo "  ninja install -C $BUILD_DIR && RUST_LOG='$RUST_LOG' $PREFIX/bin/riff"
        fi
        ;;
    release)
        setup_build "release"
        do_build
        if [[ "$INSTALL" -eq 1 ]]; then
            do_install
            echo ""
            echo "Installed. Run with: $PREFIX/bin/riff"
        else
            echo ""
            echo "Release build complete. Install with:"
            echo "  $0 release --install"
        fi
        ;;
    test)
        if [[ ! -d "$BUILD_PATH" ]]; then
            die "No build directory found. Run '$0 dev' first."
        fi
        echo "==> Running tests..."
        meson test -C "$BUILD_PATH" --verbose
        ;;
    pot)
        if [[ ! -d "$BUILD_PATH" ]]; then
            die "No build directory found. Run '$0 dev' first."
        fi
        echo "==> Regenerating pot files..."
        ninja riff-pot -C "$BUILD_PATH"
        ;;
    update-po)
        if [[ ! -d "$BUILD_PATH" ]]; then
            die "No build directory found. Run '$0 dev' first."
        fi
        echo "==> Updating .po files..."
        ninja riff-update-po -C "$BUILD_PATH"
        ;;
    cargo-src)
        if [[ ! -d "$BUILD_PATH" ]]; then
            die "No build directory found. Run '$0 dev' first."
        fi
        echo "==> Regenerating flatpak cargo sources..."
        ninja cargo-sources.json -C "$BUILD_PATH"
        ;;
    clean)
        if [[ -d "$BUILD_PATH" ]]; then
            echo "==> Removing $BUILD_PATH..."
            rm -rf "$BUILD_PATH"
            echo "Clean."
        else
            echo "Nothing to clean."
        fi
        ;;
    -h|--help|help)
        usage
        ;;
    *)
        die "Unknown command: $COMMAND. Run '$0 --help' for usage."
        ;;
esac
