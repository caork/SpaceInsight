# SpaceInsight 🚀

![SpaceInsight](img/spaceInsight.jpg)

A blazingly fast disk space analyzer built in Rust, with both a GPU GUI and a clickable terminal TUI. Designed to outperform SpaceSniffer with multi-threaded scanning and real-time treemap visualization.

![License](https://img.shields.io/badge/license-MIT-blue.svg)
![Build](https://img.shields.io/badge/build-passing-brightgreen.svg)

## Features

✨ **High-Performance Scanning**
- Multi-threaded file system crawling using `jwalk`
- Parallel processing with `rayon` for maximum CPU utilization
- Memory-efficient arena-based tree structure

🎨 **Beautiful Visualization**
- GPU-accelerated treemap rendering with `egui` and `wgpu`
- Squarified treemap algorithm for optimal aspect ratios
- Color-coded size visualization
- Real-time updates during scanning

⚡ **Optimized for Speed**
- Zero-copy data handling where possible
- Link-time optimization (LTO) for smaller binaries
- Minimal heap allocations

🖥️ **Cross-Platform**
- Windows
- macOS
- Linux

⌨️ **Terminal TUI (Linux-friendly)**
- Separate executable: `spaceinsight-tui`
- Clickable treemap tiles in terminal (mouse support)
- Split-pane interface with typed path input
- Keyboard controls for zoom/expand/collapse navigation

## Quick Start

### Installation

```bash
# Clone the repository
git clone https://github.com/yourusername/SpaceInsight.git
cd SpaceInsight

# Build the release version
cargo build --release

# Run GUI (default)
cargo run --release --bin spaceinsight

# Run Terminal TUI
cargo run --release --bin spaceinsight-tui
```

### GUI Usage

1. Launch SpaceInsight
2. Enter a directory path (or leave empty to scan current directory)
3. Click "Scan" to start the analysis
4. Watch the treemap populate in real-time!

### TUI Usage

```bash
./target/release/spaceinsight-tui
```

- Press `/` to edit path, then `Enter` to scan
- Left click on tiles to select + expand/collapse folders
- Right click folder tiles to zoom into them
- Press `u` (or Backspace) to zoom out one level
- Press `q` to quit

## Architecture

SpaceInsight is built on shared core components plus two frontends:

### 1. File Crawler (`crawler.rs`)
- Uses `jwalk` for parallel directory traversal
- Thread-safe statistics with atomic counters
- Returns a `DashMap` for lock-free concurrent access

### 2. Tree Structure (`tree.rs`)
- Arena allocator via `indextree` for memory efficiency
- Bottom-up size calculation
- Fast parent-child relationships

### 3. Treemap Layout (`treemap.rs`)
- Implements the Bruls, Huizing, and van Wijk squarified algorithm
- Recursive partitioning for optimal visualization
- Handles deep directory hierarchies without stack overflow

### 4. Shared Library (`lib.rs`)
- Re-exports scanning/layout/render modules for multiple binaries
- Keeps GUI/TUI entrypoints small and maintainable

### 5. GUI (`main.rs`)
- `egui` immediate-mode GUI
- Background thread for non-blocking scans
- Dynamic color scheme based on file sizes

### 6. TUI (`bin/spaceinsight-tui.rs`)
- `ratatui` + `crossterm` terminal UI
- Clickable text-mode treemap with split panes
- Keyboard + mouse navigation and directory drill-down

## Performance

SpaceInsight is designed to scan **1 million files in under 2 seconds** on modern hardware with:
- Multi-core CPU utilization via thread pools
- Minimal memory allocations
- Stack allocation over heap where possible

## Development

### Prerequisites

- Rust 1.70+ (2021 edition)
- Cargo

### Building from Source

```bash
# Debug build
cargo build

# Release build (optimized)
cargo build --release

# Run tests
cargo test
```

### Linux Binary Compatibility

If you see an error like:

```text
./spaceinsight: /lib/x86_64-linux-gnu/libc.so.6: version `GLIBC_2.39' not found
```

the binary was built on a newer Linux distribution than your system. Build from source on your machine:

```bash
cargo build --release
./target/release/spaceinsight
./target/release/spaceinsight-tui
```

GitHub Actions now publishes architecture-specific artifacts:

- `spaceinsight-linux-<x86_64|arm64>` (GUI, standard Linux build on Ubuntu 22.04)
- `spaceinsight-windows-<x86_64|arm64>` (GUI, standard Windows build)
- `spaceinsight-macos-<x86_64|arm64>` (GUI, standard macOS build)
- `spaceinsight-tui-linux-<x86_64|arm64>` (TUI, Linux only)
- `spaceinsight-linux-x86_64-manylinux` (GUI, compatibility build targeting `glibc 2.17`)
- `spaceinsight-tui-linux-x86_64-manylinux` (TUI, compatibility build targeting `glibc 2.17`, Linux only)

Use `spaceinsight-linux-x86_64-manylinux` if you need maximum distro compatibility.

### Project Structure

```
SpaceInsight/
├── src/
│   ├── lib.rs                  # Shared modules for all binaries
│   ├── main.rs                 # GUI application (spaceinsight)
│   ├── bin/
│   │   └── spaceinsight-tui.rs # Terminal TUI application
│   ├── crawler.rs              # File system scanner
│   ├── tree.rs                 # Hierarchical data structure
│   └── treemap.rs              # Layout algorithm
├── Cargo.toml        # Dependencies and build config
└── .github/
    └── workflows/
        └── build.yml # CI/CD pipeline
```

## Dependencies

- **eframe** - Cross-platform GUI framework
- **jwalk** - Fast parallel directory walking
- **rayon** - Data parallelism library
- **indextree** - Arena-based tree structure
- **dashmap** - Concurrent HashMap
- **serde** - Serialization framework
- **ratatui** - Terminal UI framework
- **crossterm** - Cross-platform terminal input/output

## Roadmap

- [x] Phase 1: Multi-threaded file crawler
- [x] Phase 2: Squarified treemap algorithm
- [x] Phase 3: GPU-accelerated UI
- [x] Phase 4: CI/CD pipeline
- [ ] Interactive navigation (zoom, pan, drill-down)
- [ ] File type filtering
- [ ] Export functionality
- [ ] Custom color schemes

## Contributing

Contributions are welcome! Please feel free to submit a Pull Request.

## License

This project is licensed under the MIT License - see the LICENSE file for details.

## Acknowledgments

- Inspired by [SpaceSniffer](http://www.uderzo.it/main_products/space_sniffer/)
- Treemap algorithm by Bruls, Huizing, and van Wijk
- Built with the amazing Rust ecosystem

---

**Built with ❤️ and Rust**
