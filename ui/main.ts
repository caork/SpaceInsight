import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { writeText } from "@tauri-apps/plugin-clipboard-manager";
import { open } from "@tauri-apps/plugin-dialog";
import {
  Archive,
  ArrowUp,
  ChevronRight,
  Code2,
  Copy,
  createIcons,
  Database,
  File,
  FileText,
  Film,
  Folder,
  FolderOpen,
  Image,
  Music,
  Package,
  ScanLine,
  Search,
} from "lucide";
import "./styles.css";

type PreviewItem = { path: string; name: string; size: number; isDir: boolean };
type ScanProgress = {
  phase: string;
  processedEntries: number;
  totalFiles: number;
  totalDirs: number;
  totalSize: number;
  preview: PreviewItem[];
};
type ScanComplete = {
  root: string;
  totalSize: number;
  totalFiles: number;
  totalDirs: number;
  durationMs: number;
};
type LayoutNode = {
  id: number;
  path: string;
  name: string;
  size: number;
  isDir: boolean;
  isAggregate: boolean;
  aggregateCount: number;
  x: number;
  y: number;
  width: number;
  height: number;
};
type Layout = {
  root: string;
  scannedRoot: string;
  parent: string | null;
  totalSize: number;
  totalFiles: number;
  totalDirs: number;
  nodes: LayoutNode[];
};

const app = document.querySelector<HTMLDivElement>("#app")!;
app.innerHTML = `
  <div class="app-shell">
    <header class="topbar">
      <form class="scan-form" id="scan-form">
        <label class="path-field" for="path-input">
          <i data-lucide="folder"></i>
          <input id="path-input" autocomplete="off" spellcheck="false" aria-label="Folder to scan" />
          <button class="icon-button subtle" id="browse-button" type="button" title="Choose folder" aria-label="Choose folder">
            <i data-lucide="folder-open"></i>
          </button>
        </label>
        <button class="primary-button" id="scan-button" type="submit">
          <i data-lucide="scan-line"></i><span>Scan</span>
        </button>
        <div class="scan-state" id="scan-state" hidden>
          <span class="scan-pulse"></span>
          <span id="scan-detail">Reading directory entries...</span>
          <div class="scan-track"><span id="scan-track-fill"></span></div>
        </div>
      </form>
    </header>

    <div class="workspace">
      <main class="content">
        <div class="content-toolbar">
          <button class="icon-button" id="up-button" title="Up one level" aria-label="Up one level" disabled>
            <i data-lucide="arrow-up"></i>
          </button>
          <nav class="breadcrumbs" id="breadcrumbs" aria-label="Current folder"></nav>
          <div class="toolbar-spacer"></div>
          <span class="size-summary" id="total-size">--</span>
          <span class="count-summary"><b id="file-count">--</b> files</span>
          <span class="count-summary"><b id="folder-count">--</b> folders</span>
          <span class="item-count" id="item-count"></span>
        </div>

        <section class="visualization" id="visualization">
          <div class="empty-state" id="empty-state">
            <div class="empty-visual"><i data-lucide="search"></i></div>
            <h1>See what is using your space</h1>
            <p>Choose a folder, then scan to explore its contents.</p>
          </div>
          <div class="treemap" id="treemap" hidden>
            <canvas id="treemap-canvas"></canvas>
            <div class="tile-layer" id="tile-layer"></div>
          </div>
        </section>
        <footer class="statusbar">
          <span class="status-dot" id="status-dot"></span>
          <span id="status-text">Ready</span>
          <span class="status-hint" id="status-hint">Click a folder to open it</span>
        </footer>
      </main>
    </div>
    <div class="context-menu" id="path-menu" role="menu" hidden>
      <button id="copy-path" role="menuitem"><i data-lucide="copy"></i><span>Copy path</span></button>
      <button id="open-path" role="menuitem"><i data-lucide="folder-open"></i><span>Show in file manager</span></button>
    </div>
    <div class="toast" id="toast" role="status" hidden></div>
  </div>
`;

const icons = { Archive, ArrowUp, ChevronRight, Code2, Copy, Database, File, FileText, Film, Folder, FolderOpen, Image, Music, Package, ScanLine, Search };
createIcons({ icons });

const pathInput = el<HTMLInputElement>("path-input");
const scanButton = el<HTMLButtonElement>("scan-button");
const emptyState = el("empty-state");
const scanState = el("scan-state");
const treemap = el("treemap");
const treemapCanvas = el<HTMLCanvasElement>("treemap-canvas");
const tileLayer = el("tile-layer");
const visualization = el("visualization");
const toast = el("toast");

let layout: Layout | null = null;
let selected: LayoutNode | null = null;
let resizingTimer = 0;
let scanning = false;
let contextPath = "";
let previewRequest = 0;

type FileKind = "folder" | "image" | "video" | "audio" | "archive" | "code" | "document" | "data" | "binary" | "other";

const kindColors: Record<FileKind, string> = {
  folder: "#69b5a8",
  image: "#d98e93",
  video: "#9d91ca",
  audio: "#c48ab5",
  archive: "#d9a45f",
  code: "#75a8c9",
  document: "#91b980",
  data: "#76b8bd",
  binary: "#bc8977",
  other: "#b4aa96",
};

function el<T extends HTMLElement = HTMLElement>(id: string): T {
  return document.getElementById(id) as T;
}

function formatBytes(value: number): string {
  if (!Number.isFinite(value) || value <= 0) return "0 B";
  const units = ["B", "KB", "MB", "GB", "TB", "PB"];
  const index = Math.min(Math.floor(Math.log(value) / Math.log(1024)), units.length - 1);
  const number = value / 1024 ** index;
  return `${number.toLocaleString(undefined, { maximumFractionDigits: number >= 100 ? 0 : number >= 10 ? 1 : 2 })} ${units[index]}`;
}

function formatCount(value: number): string {
  return value.toLocaleString();
}

function shortName(path: string): string {
  const parts = path.split(/[\\/]/).filter(Boolean);
  return parts.at(-1) || path;
}

function showToast(message: unknown): void {
  toast.textContent = String(message);
  toast.hidden = false;
  window.setTimeout(() => (toast.hidden = true), 3600);
}

function extensionOf(name: string): string {
  if (name.startsWith(".") && !name.slice(1).includes(".")) return name.slice(1).toLowerCase();
  return name.includes(".") ? name.split(".").at(-1)!.toLowerCase() : "";
}

function fileKind(node: LayoutNode): FileKind {
  if (node.isDir) return "folder";
  const extension = extensionOf(node.name);
  if (["png", "jpg", "jpeg", "gif", "webp", "svg", "ico", "bmp", "tif", "tiff", "heic", "raw"].includes(extension)) return "image";
  if (["mp4", "mov", "mkv", "avi", "webm", "m4v", "flv", "wmv"].includes(extension)) return "video";
  if (["mp3", "wav", "flac", "aac", "m4a", "ogg", "opus", "aiff"].includes(extension)) return "audio";
  if (["zip", "7z", "rar", "tar", "gz", "bz2", "xz", "zst", "tgz"].includes(extension)) return "archive";
  if (["rs", "ts", "tsx", "js", "jsx", "py", "go", "java", "kt", "swift", "c", "h", "cpp", "hpp", "cs", "rb", "php", "sh", "html", "css", "scss", "vue", "svelte"].includes(extension)) return "code";
  if (["pdf", "doc", "docx", "xls", "xlsx", "ppt", "pptx", "txt", "md", "rtf", "pages", "numbers", "key"].includes(extension)) return "document";
  if (["json", "jsonl", "xml", "yaml", "yml", "toml", "csv", "tsv", "sql", "db", "sqlite", "parquet", "arrow"].includes(extension)) return "data";
  if (["exe", "dll", "msi", "app", "dmg", "pkg", "deb", "rpm", "so", "dylib", "bin", "iso", "wasm", "class", "o", "a"].includes(extension)) return "binary";
  return "other";
}

function iconForKind(kind: FileKind): string {
  return ({
    folder: "folder",
    image: "image",
    video: "film",
    audio: "music",
    archive: "archive",
    code: "code-2",
    document: "file-text",
    data: "database",
    binary: "package",
    other: "file",
  })[kind];
}

function hideContextMenu(): void {
  el("path-menu").hidden = true;
}

function showContextMenu(event: MouseEvent, path: string): void {
  event.preventDefault();
  event.stopPropagation();
  contextPath = path;
  const menu = el("path-menu");
  menu.hidden = false;
  const bounds = menu.getBoundingClientRect();
  menu.style.left = `${Math.min(event.clientX, window.innerWidth - bounds.width - 8)}px`;
  menu.style.top = `${Math.min(event.clientY, window.innerHeight - bounds.height - 8)}px`;
}

async function copyPath(path: string): Promise<void> {
  try {
    await writeText(path);
    showToast("Path copied");
  } catch {
    showToast("Unable to copy the path");
  }
}

function setScanning(value: boolean): void {
  scanning = value;
  scanButton.disabled = value;
  scanButton.classList.toggle("is-loading", value);
  scanButton.querySelector("span")!.textContent = value ? "Scanning" : "Scan";
  el("status-dot").classList.toggle("active", value);
}

async function beginScan(): Promise<void> {
  if (scanning) return;
  const path = pathInput.value.trim();
  if (!path) return;
  selected = null;
  layout = null;
  updateSelection();
  emptyState.hidden = true;
  treemap.hidden = false;
  scanState.hidden = false;
  clearTreemap();
  el("scan-track-fill").style.width = "5%";
  el("status-text").textContent = "Scanning";
  setScanning(true);
  try {
    await invoke("start_scan", { path });
  } catch (error) {
    setScanning(false);
    scanState.hidden = true;
    emptyState.hidden = layout !== null;
    treemap.hidden = layout === null;
    el("status-text").textContent = "Ready";
    showToast(error);
  }
}

async function loadLayout(path: string): Promise<void> {
  const rect = visualization.getBoundingClientRect();
  const next = await invoke<Layout>("get_layout", {
    path,
    width: Math.max(320, rect.width),
    height: Math.max(320, rect.height),
  });
  layout = next;
  selected = null;
  scanState.hidden = true;
  emptyState.hidden = true;
  treemap.hidden = false;
  renderLayout();
}

function renderLayout(): void {
  if (!layout) return;
  renderNodes(layout.nodes, false);
  updateChrome();
}

function clearTreemap(): void {
  tileLayer.replaceChildren();
  const context = treemapCanvas.getContext("2d");
  context?.clearRect(0, 0, treemapCanvas.width, treemapCanvas.height);
}

function drawCanvas(nodes: LayoutNode[]): void {
  const bounds = visualization.getBoundingClientRect();
  const ratio = window.devicePixelRatio || 1;
  treemapCanvas.width = Math.max(1, Math.round(bounds.width * ratio));
  treemapCanvas.height = Math.max(1, Math.round(bounds.height * ratio));
  treemapCanvas.style.width = `${bounds.width}px`;
  treemapCanvas.style.height = `${bounds.height}px`;
  const context = treemapCanvas.getContext("2d");
  if (!context) return;
  context.setTransform(ratio, 0, 0, ratio, 0, 0);
  context.clearRect(0, 0, bounds.width, bounds.height);
  for (const node of nodes) {
    context.fillStyle = kindColors[fileKind(node)];
    const area = node.width * node.height;
    const shortSide = Math.min(node.width, node.height);
    const fade = Math.max(0.08, Math.min(1, Math.sqrt(area) / 18));
    context.globalAlpha = fade;
    const inset = shortSide >= 8 ? 0.65 : shortSide >= 3 ? 0.2 : 0;
    context.fillRect(
      node.x + inset,
      node.y + inset,
      Math.max(0.5, node.width - inset * 2),
      Math.max(0.5, node.height - inset * 2),
    );
  }
  context.globalAlpha = 1;
}

function renderNodes(nodes: LayoutNode[], isPreview: boolean): void {
  tileLayer.replaceChildren();
  drawCanvas(nodes);

  // Live scan layouts change every 100 ms. Keeping their labels in the DOM
  // leaves stale composited text in WebKit as the canvas geometry moves.
  if (isPreview) return;

  const fragment = document.createDocumentFragment();
  const interactive = nodes
    .filter((node) => node.width >= 12 && node.height >= 12 && node.width * node.height >= 196)
    .sort((a, b) => b.width * b.height - a.width * a.height)
    .slice(0, 1200);
  const interactivePaths = new Set(interactive.map((node) => node.path));
  const maxSize = Math.max(...nodes.map((node) => node.size), 1);

  nodes.forEach((node) => {
    if (!interactivePaths.has(node.path)) return;
    const kind = fileKind(node);
    const tile = document.createElement("button");
    tile.type = "button";
    tile.className = `tile kind-${kind}${node.isDir ? " directory" : " file"}`;
    tile.style.left = `${node.x}px`;
    tile.style.top = `${node.y}px`;
    tile.style.width = `${node.width}px`;
    tile.style.height = `${node.height}px`;
    tile.dataset.path = node.path;
    tile.title = `${node.name}\n${formatBytes(node.size)}`;
    tile.setAttribute("aria-label", `${node.name}, ${formatBytes(node.size)}`);

    const compact = node.width < 105 || node.height < 72;
    const tiny = node.width < 54 || node.height < 42;
    tile.classList.toggle("hit-only", tiny);
    if (!tiny) {
      const icon = document.createElement("span");
      icon.className = "tile-icon";
      icon.innerHTML = `<i data-lucide="${iconForKind(kind)}"></i>`;
      const label = document.createElement("span");
      label.className = "tile-label";
      label.textContent = node.name;
      tile.append(icon, label);
      if (!compact) {
        const size = document.createElement("span");
        size.className = "tile-size";
        size.textContent = formatBytes(node.size);
        tile.append(size);
      }
    }
    tile.style.setProperty("--weight", String(Math.max(0.16, node.size / maxSize)));
    tile.addEventListener("click", () => selectNode(node));
    tile.addEventListener("contextmenu", (event) => showContextMenu(event, node.path));
    tile.addEventListener("dblclick", () => {
      if (node.isDir) void loadLayout(node.path).catch(showToast);
      else void reveal(node.path);
    });
    fragment.append(tile);
  });
  tileLayer.append(fragment);
  createIcons({ icons });
}

function selectNode(node: LayoutNode): void {
  if (selected?.path === node.path && node.isDir && !scanning) {
    void loadLayout(node.path).catch(showToast);
    return;
  }
  selected = node;
  tileLayer.querySelectorAll(".selected").forEach((tile) => tile.classList.remove("selected"));
  tileLayer.querySelector<HTMLElement>(`[data-path="${CSS.escape(node.path)}"]`)?.classList.add("selected");
  updateSelection();
}

function updateSelection(): void {
  if (selected) {
    el("status-text").textContent = `${selected.name} · ${formatBytes(selected.size)}`;
    el("status-hint").textContent = selected.path;
  } else if (layout) {
    el("status-text").textContent = shortName(layout.root);
    el("status-hint").textContent = "Click a folder to open it";
  }
}

function updateChrome(): void {
  if (!layout) return;
  el("total-size").textContent = formatBytes(layout.totalSize);
  el("file-count").textContent = formatCount(layout.totalFiles);
  el("folder-count").textContent = formatCount(layout.totalDirs);
  el("item-count").textContent = `${formatCount(layout.nodes.length)} visible items`;
  el<HTMLButtonElement>("up-button").disabled = !layout.parent;
  el("status-text").textContent = shortName(layout.root);
  renderBreadcrumbs();
}

function renderBreadcrumbs(): void {
  if (!layout) return;
  const container = el("breadcrumbs");
  container.replaceChildren();
  const separator = layout.root.includes("\\") ? "\\" : "/";
  const rootParts = layout.scannedRoot.split(/[\\/]/).filter(Boolean);
  const parts = layout.root.split(/[\\/]/).filter(Boolean);
  const start = Math.max(0, rootParts.length - 1);
  const leading = layout.root.startsWith(separator) ? separator : "";

  parts.slice(start).forEach((part, relativeIndex) => {
    if (relativeIndex > 0) {
      const divider = document.createElement("i");
      divider.setAttribute("data-lucide", "chevron-right");
      container.append(divider);
    }
    const button = document.createElement("button");
    button.type = "button";
    button.textContent = part;
    const absoluteIndex = start + relativeIndex;
    button.addEventListener("click", () => {
      const prefix = leading + parts.slice(0, absoluteIndex + 1).join(separator);
      void loadLayout(prefix).catch(showToast);
    });
    button.addEventListener("contextmenu", (event) => {
      const prefix = leading + parts.slice(0, absoluteIndex + 1).join(separator);
      showContextMenu(event, prefix);
    });
    container.append(button);
  });
  createIcons({ icons });
}

async function reveal(path: string): Promise<void> {
  try {
    await invoke("reveal_path", { path });
  } catch (error) {
    showToast(error);
  }
}

async function renderProgress(progress: ScanProgress): Promise<void> {
  el("scan-detail").textContent = `${formatCount(progress.processedEntries)} entries · ${formatBytes(progress.totalSize)}`;
  const visualProgress = Math.min(92, 8 + Math.log10(Math.max(1, progress.processedEntries)) * 18);
  el("scan-track-fill").style.width = `${visualProgress}%`;
  el("total-size").textContent = formatBytes(progress.totalSize);
  el("file-count").textContent = formatCount(progress.totalFiles);
  el("folder-count").textContent = formatCount(progress.totalDirs);
  el("item-count").textContent = `${formatCount(progress.preview.length)} live items`;
  if (progress.preview.length === 0) return;

  const request = ++previewRequest;
  const rect = visualization.getBoundingClientRect();
  try {
    const nodes = await invoke<LayoutNode[]>("get_preview_layout", {
      preview: progress.preview,
      width: Math.max(320, rect.width),
      height: Math.max(320, rect.height),
    });
    if (request === previewRequest && scanning) renderNodes(nodes, true);
  } catch (error) {
    if (request === previewRequest) showToast(error);
  }
}

el<HTMLFormElement>("scan-form").addEventListener("submit", (event) => {
  event.preventDefault();
  void beginScan();
});
el("browse-button").addEventListener("click", async () => {
  const chosen = await open({ directory: true, multiple: false, title: "Choose a folder to scan" });
  if (typeof chosen === "string") pathInput.value = chosen;
});
el("up-button").addEventListener("click", () => {
  if (layout?.parent) void loadLayout(layout.parent).catch(showToast);
});
el("path-input").addEventListener("contextmenu", (event) => showContextMenu(event, pathInput.value.trim()));
el("copy-path").addEventListener("click", () => {
  hideContextMenu();
  if (contextPath) void copyPath(contextPath);
});
el("open-path").addEventListener("click", () => {
  hideContextMenu();
  if (contextPath) void reveal(contextPath);
});
document.addEventListener("pointerdown", (event) => {
  if (!el("path-menu").contains(event.target as Node)) hideContextMenu();
});
el("status-hint").addEventListener("contextmenu", (event) => {
  const path = selected?.path ?? layout?.root;
  if (path) showContextMenu(event, path);
});

window.addEventListener("keydown", (event) => {
  const target = event.target as HTMLElement;
  if (event.key === "Backspace" && target.tagName !== "INPUT" && layout?.parent) {
    event.preventDefault();
    void loadLayout(layout.parent).catch(showToast);
  }
  if (event.key === "Escape") hideContextMenu();
});

new ResizeObserver(() => {
  if (!layout || scanning) return;
  window.clearTimeout(resizingTimer);
  resizingTimer = window.setTimeout(() => void loadLayout(layout!.root).catch(showToast), 160);
}).observe(visualization);

void listen<ScanProgress>("scan-progress", ({ payload }) => void renderProgress(payload));
void listen<ScanComplete>("scan-complete", async ({ payload }) => {
  setScanning(false);
  pathInput.value = payload.root;
  el("scan-track-fill").style.width = "100%";
  el("status-text").textContent = `Scanned in ${(payload.durationMs / 1000).toFixed(2)}s`;
  try {
    await loadLayout(payload.root);
  } catch (error) {
    showToast(error);
  }
});

invoke<string>("default_path").then((path) => (pathInput.value = path)).catch(() => (pathInput.value = "."));
