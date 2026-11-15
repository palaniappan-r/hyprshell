#!/usr/bin/env bash
set -euo pipefail
root="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")/.." && pwd)"
target="${CARGO_TARGET_DIR:-$root/target}"
[[ "$target" = /* ]] || target="$root/$target"
config_dir="${XDG_CONFIG_HOME:-$HOME/.config}/hyprshell"
if [[ ! -x "$target/release/hyprshell" || ! -x "$target/release/hyprshell-capture" ]]; then
    echo "Run $root/scripts/build-window-previews.sh first." >&2
    exit 1
fi
# Preserve the local integrated-GPU and GTK renderer settings.
unset __NV_PRIME_RENDER_OFFLOAD __NV_PRIME_RENDER_OFFLOAD_PROVIDER
unset __GLX_VENDOR_LIBRARY_NAME __VK_LAYER_NV_optimus
export DRI_PRIME=pci-0000_65_00_0
export __EGL_VENDOR_LIBRARY_FILENAMES=/usr/share/glvnd/egl_vendor.d/50_mesa.json
export GSK_RENDERER=opengl
export GDK_BACKEND=wayland
export HYPRSHELL_PREVIEW_HELPER="$target/release/hyprshell-capture"
exec "$target/release/hyprshell" --config-file "$config_dir/config.ron" \
    --css-file "$config_dir/styles.css" run "$@"
