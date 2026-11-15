# Frozen window previews

The window switcher includes snapshot previews integrated directly into the
Rust sources. There is no source-patching step. Source, scripts, binaries, and
build artifacts live in this repository; `~/.config/hyprshell` contains only
configuration, styles, and their backups.

Each window card starts capturing one frame as the switcher opens. The last screenshot for each window
is shown immediately while a fresh frame loads. Windows without a cached
frame use a plain dark thumbnail; there is no large icon placeholder.
Capture startup and reads run on four background workers, limiting concurrent
compositor readbacks. Quick releases cancel both queued and active captures.
The Wayland helper uses Hyprland's toplevel export protocol, sends a scaled
RGBA frame, and exits. GTK keeps the snapshot in memory, with no continuous
capture or refresh loop. Navigation and relayouts reuse the same snapshot,
including after another window is closed. Closing clears the current capture
session and retains each window's last good image for the next opening.
Cached images for closed windows are pruned when the switcher opens. If a
capture fails, the previous image or dark thumbnail stays visible.

Cards have a maximum content size of 360×225 logical pixels. The grid fits the
focused monitor and reduces card sizes when needed. Captures use up to two
physical pixels per logical pixel, capped at 720×450. Workspace badges and
close actions are preserved.

Super+Tab focuses the next window and opens the popup immediately, with no
intentional show delay or fades. Navigation highlights previews and releasing
Super focuses the selection. The held-Super session keeps a fixed window order.
Switcher keybindings send native compositor events over one subscribed socket,
preserving press/release order without launching a helper process for each key.
This prevents a late open message from arriving after release and opening a
short-lived popup. The existing `hyprshell socat` interface remains available.
Closing hides the popup immediately and releases its current snapshots and
unfinished captures. Escape restores the window that was active
before the session. Each title has a 16 px application icon to its left. A
switcher configured without a modifier still opens the popup immediately.

## Build and run

In addition to the normal build dependencies, the helper needs a C compiler,
Python 3, `pkg-config`, Wayland client development files, `wayland-scanner`, and
the installed `hyprland-protocols` toplevel-export XML.

From the repository root:

```sh
bash scripts/build-window-previews.sh
```

The script builds and tests the C helper, then builds Hyprshell using the
locked, locally cached Cargo dependencies. The resulting
`target/release/hyprshell` and `target/release/hyprshell-capture` must be kept
next to each other. `HYPRSHELL_PREVIEW_HELPER` can override the helper path.
`CARGO_TARGET_DIR` can override the output directory.

The copied configuration and stylesheet reproduce the local preview layout:

```sh
target/release/hyprshell \
  --config-file scripts/window-previews/config.ron \
  --css-file scripts/window-previews/styles.css run
```

For the installed local configuration and integrated-GPU settings, run:

```sh
~/Repos/hyprshell/scripts/start-window-previews.sh
```

The launcher reads `config.ron` and `styles.css` from
`${XDG_CONFIG_HOME:-$HOME/.config}/hyprshell` and runs the binaries in this
repository's `target/release` directory (or `CARGO_TARGET_DIR`). The Hyprland
startup wrapper at `~/.config/hypr/start-hyprshell-igpu` calls this launcher.
Run one Hyprshell daemon at a time. Building does not restart the daemon.

The former configuration-directory scripts, source patcher, binaries, and
build tree are preserved in `target/legacy-window-previews/` as a historical
archive. Those scripts retain their old paths; use the repository scripts
above for future builds and launches. The archive is ignored by Git and can
be removed when no longer needed.

## Files and checks

- `crates/windows-lib/src/preview.rs`: capture lifecycle and per-opening cache.
- `crates/windows-lib/src/preview/frame.rs`: bounded frame decoding and tests.
- `crates/windows-lib/src/preview/layout.rs`: monitor-aware sizing and tests.
- `crates/windows-lib/src/switch/`: preview cards, shared cache, and navigation.
- `scripts/window-previews/`: capture helper, pixel-conversion test, and local
  style/configuration.

```sh
cargo test --locked --offline -p hyprshell-windows-lib --lib preview::
bash scripts/build-window-previews.sh --helper-only
```

The helper's pixel-format, padding, alpha, and inverted-frame tests run without
a compositor. Actual window capture needs a running Hyprland session.
