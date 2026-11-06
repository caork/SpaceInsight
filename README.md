# SpaceInsight

A disk space analyzer with a **native, GPU-rendered GPUI desktop interface** and an optional terminal UI. The desktop is Rust throughout: no Tauri, WebView, Node.js, or frontend build step.

## Desktop

```sh
cargo run --locked -p spaceinsight-desktop --bin spaceinsight
# Optional initial path (press Scan after launch):
cargo run --locked -p spaceinsight-desktop --bin spaceinsight -- /path/to/folder
```

Choose a folder or type/paste its path, then press **Scan** or Enter. The background scanner publishes a live top-level treemap preview. Click a folder to drill down; use breadcrumbs, **Up**, or Alt+Up to navigate within the scan. Click a file or right-click any tile to select it, then use **Copy path** or **Show in files**. Cmd/Ctrl+O opens the folder picker; Cmd/Ctrl+Q quits. Navigation pauses during scanning. Rescan to refresh changed files.

Requirements:

- Current stable Rust (GPUI uses the 2024 edition); GPUI is pinned to `0.2.2` and dependencies are locked.
- macOS: Xcode, Command Line Tools, and the Metal compiler (`xcodebuild -downloadComponent MetalToolchain`).
- Linux: a working GPU driver, X11 or Wayland, and development packages. On Ubuntu: `sudo apt-get install libxcb-render0-dev libxcb-shape0-dev libxcb-xfixes0-dev libxkbcommon-dev libxkbcommon-x11-dev libssl-dev libwayland-dev libvulkan-dev`.
- Windows: the MSVC Rust toolchain and Visual Studio C++ build tools; a DirectX 11 capable graphics driver.

GPUI paints its own controls in a native window, rather than using system widgets. Its platform requirements apply; mobile and browser targets are not supported by this application. Linux and Windows builds are covered by CI configuration; they must pass on their target runners before being treated as verified.

```sh
cargo build --release --workspace --locked
# macOS app bundle, with the existing application icon:
./scripts/bundle-macos.sh
open target/release/SpaceInsight.app
```

The bundle script creates a locally ad-hoc signed application. Developer ID signing, notarization, Windows installers, and Linux packaging are separate distribution steps; the old Tauri installer pipeline has been removed. No glibc 2.17 compatibility is claimed for the GPU desktop.

## Terminal UI

```sh
cargo run --release --locked --bin spaceinsight-tui
```

- `/` edits the path; Enter scans.
- Left-click selects/expands a folder; right-click zooms in.
- `u` or Backspace moves up; `q` quits.

## Architecture

- `src/`: shared parallel `jwalk`/`rayon` scanner, arena-backed file tree, squarified treemap layout, and terminal support.
- `desktop/src/main.rs`: GPUI views, GPU canvas, hit testing, navigation, clipboard, and native folder picker/file-manager integration.
- `desktop/src/scan.rs`: background scanning and tree construction; coalesced progress prevents queued updates from growing with scan duration.
- `desktop/src/input.rs`: editable path with selection, clipboard, Unicode boundaries, and platform IME integration, adapted from GPUI's input example. See `desktop/NOTICE` and `desktop/LICENSE-APACHE`.
- `scripts/bundle-macos.sh`: macOS application bundle.

Both desktop and terminal reuse the shared Rust scanning/layout core. File-system work and tree construction run off the UI thread. On macOS/Linux, the scanner sums allocated file blocks (512 bytes per `st_blocks` unit), counts hard-linked storage once, and skips symlinks/devices. Sparse and compressed files are charged for allocated bytes, including Docker VM images. Windows currently falls back to logical file length and the UI labels this explicitly. Desktop units use decimal KB/MB/GB, matching macOS Storage.

For a macOS boot-root scan, `/System/Volumes`, `/Volumes`, and `/dev` are excluded: the secondary APFS namespace duplicates the unified root view and external disks/devices are outside the boot-volume scope. Explicitly selecting a volume under those locations still scans it. Unreadable entry counts are reported.

The displayed **Allocated** total is the sum of readable file allocations, not the system's used-space meter: protected content, snapshots, filesystem metadata, purgeable storage and APFS clone/shared extents can cause differences. Shared clone extents are not deduplicated. Empty/fully sparse files have zero area in the treemap. You can audit scanner/progress/tree totals with `cargo run --release --example scan-summary -- /path/to/folder`. Scans currently cannot be cancelled. Custom controls and the canvas do not yet expose a complete accessibility tree, and keyboard-only tile navigation is not implemented. Platform IME hooks are present; full interactive IME composition still needs manual validation. Closing the last window quits the app.

## Verification

```sh
cargo test --workspace --locked
cargo check --workspace --all-targets --locked
cargo clippy --locked -p spaceinsight-desktop --all-targets --no-deps -- -D warnings
```

## License

MIT for SpaceInsight code; the adapted text-input implementation retains GPUI's Apache-2.0 terms.
