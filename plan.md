

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


# UIUX
1) 必须“更接近正方形”的形状约束（禁止长条）
	•	所有可见块（文件夹/文件）必须为正方形或强正方形倾向的近似正方形，严禁出现明显长条。
	•	硬约束（Hard constraint）：任意块的长宽比 max(w,h)/min(w,h) 不得超过 1.6（可调，但必须有上限）。
	•	若严格面积比例导致无法满足长宽比上限，允许采用下列方式之一（必须选一种并实现一致）：
	•	方案A：面积量化（推荐）：将画布离散为 N×N 网格（如 200×200 或随画布大小自适应），每个项目占用若干网格单元；面积按大小映射到单元数，允许小误差（例如 ±3% 或按最小单元粒度）。
	•	方案B：留白容错：保持面积严格比例，但允许极少量不可避免的细缝/空隙（上限如 <1% 画布面积），以换取正方形形态。
	•	布局算法要求：必须使用“square packing / squarified”策略或等效实现，目标是让块尽可能接近正方形，并尽量填满容器。

2) 画布填满规则（整体与局部容器都要填满）
	•	顶层画布为正方形，所有块应共同填满整个正方形画布（允许极细 gutter，但不得出现明显空洞）。
	•	文件夹展开后，其内部子块也必须填满该文件夹块的内部区域（同样遵循正方形倾向 + 填满）。

3) 单击/双击交互（层级“子集”展示规则）
	•	单击（Single Click）文件夹块的主体区域：执行“裂开/展开（split open）”
	•	展开后显示该文件夹的第一层子集（直接子级：子文件夹 + 文件）。
	•	展开过程需有平滑过渡（可选），但最终布局必须稳定、可点击。
	•	双击（Double Click）文件夹块的主体区域：执行“更深一层的子集展示（subset drill）”
	•	规则：在当前展开基础上，将展开内容切换为更深一层（例如显示孙辈，或以“递归展开到深度=2”的子集）。
	•	必须明确：双击不会无限递归到所有后代；它只增加预定义的深度/范围（例如深度+1，最大深度可配置）。
	•	双击后的布局仍需满足“更接近正方形”的形状约束。
	•	文件（File）块交互：
	•	文件始终显示为块，但不可裂开。
	•	单击/双击文件块不得触发展开（可选：选中/高亮/显示详情）。

4) 折叠规则（点击展开文件夹的边框折叠）
	•	当文件夹处于展开状态时，必须存在可见边框。
	•	点击边框区域：立即折叠（collapse），恢复为单块（仅表示该文件夹总大小），内部子块全部消失。
	•	折叠应优先于内部块点击（边框命中区域的事件优先级更高）。

5) 边框命中区域（厚度要好点但不抢面积）
	•	边框需要易点击的命中厚度，但视觉上不要太粗：
	•	可见边框：细线（例如 1–2px）
	•	命中区域：在边框内/外扩展一个透明点击带（例如 6–10px，可调）
	•	命中区域不得过宽，以免显著吞噬内容面积；命中带与内容区要有清晰分界（避免误触内部块）。

6) 标签策略（你要求：小块不显示，hover 才显示）
	•	默认不强制显示所有文件名/文件夹名。
	•	当块面积小于阈值（例如不足以容纳一行文字）时：
	•	不显示文字标签（保持干净）。
	•	Hover 必须显示完整名称（tooltip/popover），并可同时显示大小信息。
	•	对于足够大的块：可以显示简短名称；过长则省略号；hover 展示全名（但这不是必须覆盖所有块的强需求）。

7) 面积编码（核心不变）
	•	块面积必须映射大小：
	•	文件夹块面积代表该文件夹总大小（含内部）。
	•	文件块面积代表文件大小。
	•	若采用“网格量化方案”，需声明：面积与大小保持单调一致，并将误差限制在可接受范围内（例如按最小网格粒度）。