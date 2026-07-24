#!/usr/bin/env bash
set -euo pipefail

usage() {
    cat <<'EOF'
Install Talkdown for the current user.

Usage: scripts/install.sh [--prefix DIR] [--no-build] [--features FEATURES]

Options:
  --prefix DIR        Installation prefix (default: $PREFIX or ~/.local)
  --no-build          Install an existing target/release/talkdown binary
  --features LIST     Cargo feature list, such as whisper-cuda or whisper-vulkan
  -h, --help          Show this help
EOF
}

repository_dir="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")/.." && pwd)"
install_prefix="${PREFIX:-${HOME}/.local}"
build=true
cargo_features=""

while (($# > 0)); do
    case "$1" in
        --prefix)
            if (($# < 2)); then
                echo "error: --prefix needs a directory" >&2
                exit 2
            fi
            install_prefix="$2"
            shift 2
            ;;
        --no-build)
            build=false
            shift
            ;;
        --features)
            if (($# < 2)); then
                echo "error: --features needs a Cargo feature list" >&2
                exit 2
            fi
            cargo_features="$2"
            shift 2
            ;;
        -h|--help)
            usage
            exit 0
            ;;
        *)
            echo "error: unknown option: $1" >&2
            usage >&2
            exit 2
            ;;
    esac
done

if [[ "$(uname -s)" != "Linux" ]]; then
    echo "error: this installer currently supports Linux desktop environments only" >&2
    exit 1
fi
if [[ -z "$install_prefix" || "$install_prefix" != /* || "$install_prefix" == "/" ]]; then
    echo "error: --prefix must be a non-root absolute directory" >&2
    exit 2
fi
install_prefix="${install_prefix%/}"

cd -- "$repository_dir"

if [[ "$build" == true ]]; then
    cargo_arguments=(build --release)
    if [[ -n "$cargo_features" ]]; then
        cargo_arguments+=(--features "$cargo_features")
    fi
    cargo "${cargo_arguments[@]}"
fi

binary_source="${CARGO_TARGET_DIR:-${repository_dir}/target}/release/talkdown"
if [[ "$binary_source" != /* ]]; then
    binary_source="${repository_dir}/${binary_source}"
fi
if [[ ! -x "$binary_source" ]]; then
    echo "error: release binary not found at $binary_source" >&2
    echo "Run this script without --no-build, or set CARGO_TARGET_DIR correctly." >&2
    exit 1
fi

binary_destination="${install_prefix}/bin/talkdown"
application_dir="${install_prefix}/share/applications"
icon_dir="${install_prefix}/share/icons/hicolor/scalable/apps"
desktop_destination="${application_dir}/talkdown.desktop"
icon_destination="${icon_dir}/talkdown.svg"

install -d -- "${install_prefix}/bin" "$application_dir" "$icon_dir"
install -m 0755 -- "$binary_source" "$binary_destination"
install -m 0644 -- assets/talkdown.svg "$icon_destination"

desktop_exec="$binary_destination"
desktop_exec="${desktop_exec//\\/\\\\}"
desktop_exec="${desktop_exec//\"/\\\"}"
desktop_exec="${desktop_exec//\`/\\\`}"
desktop_exec="${desktop_exec//\$/\\$}"
desktop_exec="${desktop_exec//%/%%}"

desktop_temp="$(mktemp --suffix=.desktop "${TMPDIR:-/tmp}/talkdown-desktop.XXXXXX")"
cleanup() {
    rm -f -- "$desktop_temp"
}
trap cleanup EXIT

while IFS= read -r line || [[ -n "$line" ]]; do
    if [[ "$line" == Exec=* ]]; then
        printf 'Exec="%s" %%f\n' "$desktop_exec"
    else
        printf '%s\n' "$line"
    fi
done < packaging/linux/talkdown.desktop > "$desktop_temp"

if command -v desktop-file-validate >/dev/null 2>&1; then
    desktop-file-validate "$desktop_temp"
fi
install -m 0644 -- "$desktop_temp" "$desktop_destination"

if command -v update-desktop-database >/dev/null 2>&1; then
    update-desktop-database "$application_dir"
fi

echo "Talkdown installed:"
echo "  application: $binary_destination"
echo "  desktop file: $desktop_destination"
echo "  icon: $icon_destination"
