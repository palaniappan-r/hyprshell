#!/usr/bin/env bash
# Build the integrated frozen previews and their one-frame Wayland helper.
set -euo pipefail
root="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$root"
helper_only=false
case "${1:-}" in
    '') ;;
    --helper-only) helper_only=true ;;
    *) echo "Usage: $0 [--helper-only]" >&2; exit 2 ;;
esac
if (( $# > 1 )); then echo "Usage: $0 [--helper-only]" >&2; exit 2; fi
target="${CARGO_TARGET_DIR:-$root/target}"
build_dir="$target/window-preview"
protocol=/usr/share/hyprland-protocols/protocols/hyprland-toplevel-export-v1.xml
mkdir -p "$build_dir" "$target/release"
python3 - "$protocol" "$build_dir/export.xml" <<'PY'
import sys
import xml.etree.ElementTree as ET

tree = ET.parse(sys.argv[1])
manager = tree.find("interface[@name='hyprland_toplevel_export_manager_v1']")
# Numeric window addresses use v1; omit the v2 foreign-toplevel dependency.
manager.set('version', '1')
for request in list(manager.findall('request')):
    if int(request.get('since', '1')) > 1:
        manager.remove(request)
tree.write(sys.argv[2], encoding='utf-8', xml_declaration=True)
PY
wayland-scanner client-header "$build_dir/export.xml" "$build_dir/toplevel-export.h"
wayland-scanner private-code "$build_dir/export.xml" "$build_dir/toplevel-export.c"
read -r -a wayland_flags <<< "$(pkg-config --cflags --libs wayland-client)"
cc -std=c11 -O2 -Wall -Wextra -Werror \
    "$root/scripts/window-previews/capture.c" "$build_dir/toplevel-export.c" \
    -I"$build_dir" "${wayland_flags[@]}" -o "$target/release/hyprshell-capture"
cc -std=c11 -O2 -Wall -Wextra -Werror \
    "$root/scripts/window-previews/test-capture.c" "$build_dir/toplevel-export.c" \
    -I"$build_dir" "${wayland_flags[@]}" -o "$build_dir/test-capture"
"$build_dir/test-capture"
if [[ "$helper_only" == false ]]; then
    cargo build --release --locked --offline
fi
