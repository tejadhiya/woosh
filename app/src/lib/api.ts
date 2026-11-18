import { invoke } from "@tauri-apps/api/core";
import { listen, type UnlistenFn } from "@tauri-apps/api/event";
import { revealItemInDir } from "@tauri-apps/plugin-opener";

// ---- types mirrored from the engine ----

export type Risk = "safe" | "caution" | "advisory";
export type DomainKey =
  | "venv" | "node_modules" | "global_python"
  | "ai_model" | "docker" | "cache" | "artifact" | "large_file";

export interface Finding {
  id: string;
  domain: DomainKey;
  name: string;
  path: string | null;
  detail: string;
  bytes: number;
  shared_bytes: number;
  files: number;
  mtime: number;
  days_idle: number;
  risk: Risk;
  restore: string | null;
  meta?: Record<string, unknown>;
}

export interface DiskStats {
  mount: string;
  total: number;
  used: number;
  free: number;
  percent_used: number;
  arch: string;
  hostname: string;
}

export interface Totals {
  reclaimable: number;
  shared: number;
  items: number;
  by_domain: [string, number, number][];
}

export interface ScanResult {
  disk: DiskStats;
  findings: Finding[];
  totals: Totals;
  denied: string[];
  duration_us: number;
  entries_scanned: number;
  roots: string[];
}

export type LineKind = "boot" | "scan" | "found" | "woosh" | "warn" | "done";
export type Phase = "boot" | "discover" | "measure" | "probe" | "reap" | "done";

export interface LogLine {
  t_us: number;
  kind: LineKind;
  glyph: string;
  text: string;
}

export interface Frame {
  seq: number;
  elapsed_us: number;
  phase: Phase;
  entries: number;
  dirs: number;
  bytes: number;
  rate: number;
  denied: number;
  findings: number;
  paths: string[];
  lines: LogLine[];
}

export interface Receipt {
  id: string;
  name: string;
  path: string | null;
  domain: DomainKey;
  bytes: number;
  success: boolean;
  method: string;
  error: string | null;
  manifest: { file_path: string; package_count: number; python_version: string } | null;
  timestamp: string;
  micros: number;
}

export interface Summary {
  timestamp: string;
  mode: string;
  total: number;
  succeeded: number;
  failed: number;
  reclaimed: number;
  receipts: Receipt[];
}

export interface Permissions {
  full_disk_access: boolean;
  home: string;
}

export interface CachedScan {
  timestamp: number;
  fsevent_id: number;
  scan_result: ScanResult;
}

export type ReapMode = "trash" | "permanent" | "dry_run";

// ---- commands ----

export const api = {
  permissions: () => invoke<Permissions>("permissions"),
  disk: () => invoke<DiskStats>("disk"),
  scanRoots: () => invoke<string[]>("scan_roots"),
  scan: (deep: boolean) => invoke<ScanResult>("scan", { deep }),
  loadCache: () => invoke<CachedScan | null>("load_cached_scan"),
  reap: (ids: string[], mode: ReapMode) =>
    invoke<Summary>("reap", { req: { ids, mode } }),
  putBack: (receipt: Receipt) => invoke<void>("put_back", { receipt }),
  history: (limit = 25) => invoke<Summary[]>("history", { limit }),
  quickLook: (path: string) => invoke<void>("quick_look", { path }),
  reveal: (path: string) => revealItemInDir(path),
  openPath: (path: string) => invoke<void>("open_path", { path }),
};

export const onTelemetry = (cb: (f: Frame) => void): Promise<UnlistenFn> =>
  listen<Frame>("telemetry", (e) => cb(e.payload));

export const onFinding = (cb: (f: Finding) => void): Promise<UnlistenFn> =>
  listen<Finding>("finding", (e) => cb(e.payload));

export const onReceipt = (cb: (r: Receipt) => void): Promise<UnlistenFn> =>
  listen<Receipt>("reap", (e) => cb(e.payload));

// ---- presentation helpers ----

export const DOMAIN_LABEL: Record<DomainKey, string> = {
  venv: "Virtual environments",
  node_modules: "node_modules",
  global_python: "Global Python",
  ai_model: "AI & ML models",
  docker: "Docker",
  cache: "Toolchain caches",
  artifact: "Build artifacts",
  large_file: "Large files",
};

export const DOMAIN_GLYPH: Record<DomainKey, string> = {
  venv: "◆",
  node_modules: "⬢",
  global_python: "◇",
  ai_model: "✶",
  docker: "▣",
  cache: "▤",
  artifact: "▸",
  large_file: "◈",
};
