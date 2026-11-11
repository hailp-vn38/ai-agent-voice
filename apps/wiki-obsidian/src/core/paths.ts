import type { Collection, Layout } from "./types";

/** Normalize a model-supplied path. Returns null when it could escape the root. */
export function normalizePath(raw: string): string | null {
  const cleaned = raw.trim().replace(/\\/g, "/").replace(/^(\.\/)+/, "").replace(/^\/+/, "");
  if (!cleaned || /[\u0000-\u001f]/.test(cleaned)) return null;
  const parts = cleaned.split("/");
  if (parts.some((part) => part === "" || part === "." || part === "..")) return null;
  return parts.join("/");
}

export function withMd(path: string): string {
  return path.toLowerCase().endsWith(".md") ? path : `${path}.md`;
}

export function stripMd(path: string): string {
  return path.toLowerCase().endsWith(".md") ? path.slice(0, -3) : path;
}

export function fileName(path: string): string {
  return path.slice(path.lastIndexOf("/") + 1);
}

export function dirOf(layout: Layout, collection: Collection): string {
  return collection === "wiki" ? layout.wikiDir : layout.sourcesDir;
}

export function collectionOf(layout: Layout, path: string): Collection | null {
  if (path.startsWith(`${layout.wikiDir}/`)) return "wiki";
  if (path.startsWith(`${layout.sourcesDir}/`)) return "sources";
  return null;
}

/**
 * Turn `rel_path` + `scope` from a tool call into a root-relative `.md` path.
 * Accepts the path with or without the collection folder in front.
 */
export function scopedPath(layout: Layout, collection: Collection, rel: string): string | null {
  const normalized = normalizePath(rel);
  if (!normalized) return null;
  const dir = dirOf(layout, collection);
  const inner = normalized.startsWith(`${dir}/`) ? normalized.slice(dir.length + 1) : normalized;
  if (!inner) return null;
  return withMd(`${dir}/${inner}`);
}

/** Path of a source inside `sources/`, without that folder in front. */
export function sourceInnerPath(layout: Layout, rel: string): string | null {
  const full = scopedPath(layout, "sources", rel);
  return full ? full.slice(layout.sourcesDir.length + 1) : null;
}
