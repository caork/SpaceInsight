# SpaceInsight

SpaceInsight is a fast, cross-platform disk space analyzer. Its Rust scanning
engine is paired with a Tauri desktop interface and an optional terminal UI.

## Highlights

- Parallel file-system scanning with `jwalk` and `rayon`
- Squarified treemap layout calculated by the shared Rust core
- Responsive Tauri 2 desktop interface with folder drill-down
- Live scan progress and top-level preview
- Native folder picker and file-manager reveal actions
- Separate mouse-enabled terminal UI

## Desktop App

### Requirements

- Rust stable
- Node.js 20 or newer
- The [Tauri system prerequisites](https://v2.tauri.app/start/prerequisites/)
  for your platform

### Development

```bash
npm install
npm run tauri dev
```

The Vite frontend is also available at `http://localhost:1420` while Tauri is
running, but file-system commands require the desktop window.

### Production Build

```bash
npm run tauri build
```

Installers and application bundles are written beneath
`src-tauri/target/release/bundle/`.

## Terminal UI

```bash
cargo run --release --bin spaceinsight-tui
```

- Press `/` to edit the path and `Enter` to scan
- Left-click tiles to select or expand folders
- Right-click a folder to zoom in
- Press `u` or Backspace to move up
- Press `q` to quit

## Architecture

```text
SpaceInsight/
├── src/                    Rust scanning, tree, and layout core
│   └── bin/                Terminal UI entrypoint
├── src-tauri/              Tauri commands and desktop configuration
├── ui/                     TypeScript interface and styles
├── index.html              Vite entrypoint
└── package.json            Frontend and Tauri scripts
```

The Tauri backend keeps the scanned `FileTree` in memory. The frontend requests
layouts for the current directory, while `build_render_tree` and
`SquarifiedTreemap` remain the single source of truth for tile geometry.

## Verification

```bash
cargo test
npm run build
cargo check --manifest-path src-tauri/Cargo.toml
```

## License

MIT
