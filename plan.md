

## 1. The Core Architecture

To beat SpaceSniffer, the AI must prioritize **concurrency** and **zero-copy data handling**.

| Layer | Responsibility | Recommended Tech |
| --- | --- | --- |
| **Engine** | High-speed, parallel file system crawling | `jwalk` (faster than `walkdir`) |
| **Data Store** | Memory-efficient tree structure | `ego-tree` or custom `Arena` based tree |
| **UI Framework** | Cross-platform, GPU-accelerated interface | **egui** (Immediate mode, very low overhead) |
| **Rendering** | Hardware-accelerated Treemap layout | **wgpu** (backend for egui) |
| **Concurrency** | Work stealing for multi-core scanning | `rayon` |

---

## 2. Project Milestones (Agent Roadmap)

### Phase 1: The "Engine" (Foundation)

* **Goal:** Create a CLI-based crawler that can scan 1 million files in under 2 seconds.
* **Agent Task:** "Implement a multi-threaded file crawler using `jwalk`. Store results in a hierarchical tree. Use `dashmap` for thread-safe metadata collection. Target: Minimal RAM usage (use `SmallVec` for file paths)."

### Phase 2: The Treemap Logic

* **Goal:** Calculate the "Squarified Treemap" layout.
* **Agent Task:** "Implement the Bruls, Huizing, and van Wijk algorithm for squarified treemaps. It must take a parent rectangle and a list of node sizes, returning a list of child rectangles. Ensure it handles deep recursion without stack overflow."

### Phase 3: The GPU UI (Visuals)

* **Goal:** Render the rectangles using `egui`.
* **Agent Task:** "Setup an `eframe` (egui) window. Draw the treemap using `Shape::Rect`. Add a 'hot-reloading' scan feature where the UI updates as the crawler finds new files."

### Phase 4: CI/CD & Performance Tuning

* **Goal:** Automate builds for Windows, Mac, and Linux.
* **Agent Task:** "Create a GitHub Action `.yml` that compiles for `x86_64-pc-windows-msvc`, `x86_64-unknown-linux-gnu`, and `aarch64-apple-darwin`. Enable `LTO` (Link Time Optimization) in `Cargo.toml`."

---

## 3. AI Agent Instructions (`.cursorrules`)

Copy and paste this into your project’s `.cursorrules` or system prompt to keep the AI on track:

> **Role:** You are a Senior Rust Systems Engineer specializing in high-performance GUI tools.
> **Coding Standards:**
> * Prioritize **stack allocation** over heap allocation where possible.
> * Avoid `String` clones; use `Cow<str>` or `Arc<str>` for path fragments to save memory.
> * Use `Rayon` for all heavy computational loops (scanning, layout calculation).
> * Ensure the UI remains responsive (60fps) even during 100% CPU disk scanning.
> * All GUI code must be cross-platform (no `winapi` direct calls).
> 
> 

---

## 4. GitHub Workflow Blueprint

Save this as `.github/workflows/build.yml` for the agent to refine:

```yaml
name: Cross-Platform Release
on: [push, pull_request]

jobs:
  build:
    name: Build on ${{ matrix.os }}
    runs-on: ${{ matrix.os }}
    strategy:
      matrix:
        os: [ubuntu-latest, windows-latest, macos-latest]
    steps:
      - uses: actions/checkout@v4
      - name: Install Rust
        run: rustup update stable
      - name: Build Release
        run: cargo build --release
      - name: Upload Artifact
        uses: actions/upload-artifact@v4
        with:
          name: binary-${{ matrix.os }}
          path: target/release/your_app_name*

```

---
suggestion:
[dependencies]
eframe = "0.26" # Modern GPU-backed UI
jwalk = "0.8"   # Fast parallel walking
rayon = "1.8"   # Multi-threading
indextree = "4.6" # High-perf Arena tree
serde = { version = "1.0", features = ["derive"] }

[profile.release]
lto = "fat"         # Heavy optimization
codegen-units = 1   # Better performance
panic = "abort"     # Smaller binary size
