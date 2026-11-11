import { parseDoc, asString } from "./frontmatter";
import { generateIndexMd } from "./indexMd";
import { appendLogEntry } from "./logMd";
import { collectionOf, fileName, normalizePath, stripMd, withMd } from "./paths";
import type { Collection, Layout, VaultPort } from "./types";

export interface WikiLink {
  target: string;
  line: number;
}

export interface WikiDoc {
  path: string;
  collection: Collection;
  title: string;
  data: Record<string, unknown>;
  hasFrontmatter: boolean;
  text: string;
  body: string;
  links: WikiLink[];
  mtime: number;
  /** Folded term → weighted frequency, for BM25. */
  terms: Map<string, number>;
  length: number;
}

export interface SearchHit {
  path: string;
  title: string;
  score: number;
  line: number;
  snippet: string;
}

export interface LinkGraph {
  outgoing: Map<string, Set<string>>;
  inbound: Map<string, Set<string>>;
  broken: { path: string; target: string; line: number }[];
}

const WIKILINK = /\[\[([^\]|#\n]+)(?:#[^\]|\n]*)?(?:\|[^\]\n]*)?\]\]/g;
const FENCE = /^\s*(```|~~~)/;
const BM25_K1 = 1.2;
const BM25_B = 0.75;

/** Lowercase and strip diacritics so "chien luoc" finds "chiến lược". */
export function tokenize(text: string): string[] {
  const folded = text.toLowerCase().normalize("NFD").replace(/\p{M}+/gu, "").replace(/đ/g, "d");
  return folded.match(/[\p{L}\p{N}]+/gu) ?? [];
}

export function extractLinks(text: string): WikiLink[] {
  const links: WikiLink[] = [];
  let fenced = false;
  text.split("\n").forEach((line, index) => {
    if (FENCE.test(line)) {
      fenced = !fenced;
      return;
    }
    if (fenced) return;
    for (const match of line.matchAll(WIKILINK)) {
      const target = (match[1] as string).trim();
      if (target) links.push({ target, line: index + 1 });
    }
  });
  return links;
}

function isAssetTarget(target: string): boolean {
  const ext = /\.([A-Za-z0-9]{1,5})$/.exec(target)?.[1]?.toLowerCase();
  return ext !== undefined && ext !== "md";
}

/**
 * In-memory picture of the wiki: every page parsed once, kept current by
 * `refresh`, and queried for search, links and bookkeeping files.
 */
export class WikiStore {
  private docs = new Map<string, WikiDoc>();
  private loading: Promise<void> | null = null;
  private lookup: { byLower: Map<string, string>; byStem: Map<string, string[]> } | null = null;

  constructor(
    readonly vault: VaultPort,
    readonly layout: Layout,
    readonly now: () => Date = () => new Date(),
  ) {}

  get indexPath(): string {
    return `${this.layout.wikiDir}/index.md`;
  }

  get logPath(): string {
    return `${this.layout.wikiDir}/log.md`;
  }

  ensureLoaded(): Promise<void> {
    this.loading ??= (async () => {
      for (const path of await this.vault.list()) await this.refresh(path);
    })();
    return this.loading;
  }

  /** Re-read one file after it changed outside the store. */
  async refresh(path: string): Promise<void> {
    const collection = collectionOf(this.layout, path);
    if (!collection || !path.toLowerCase().endsWith(".md")) return;
    const text = await this.vault.read(path);
    if (text === null) this.forget(path);
    else this.put(path, collection, text, this.vault.mtime(path) ?? this.now().getTime());
  }

  forget(path: string): void {
    if (this.docs.delete(path)) this.lookup = null;
  }

  private put(path: string, collection: Collection, text: string, mtime: number): void {
    const parsed = parseDoc(text);
    const heading = /^#\s+(.+)$/m.exec(parsed.body)?.[1]?.trim();
    const stem = stripMd(fileName(path));
    const title = asString(parsed.data.title) ?? heading ?? stem;
    const terms = new Map<string, number>();
    const add = (source: string, weight: number) => {
      for (const term of tokenize(source)) terms.set(term, (terms.get(term) ?? 0) + weight);
    };
    add(parsed.body, 1);
    add(title, 3);
    add(stem, 2);
    let length = 0;
    for (const count of terms.values()) length += count;
    if (!this.docs.has(path)) this.lookup = null;
    this.docs.set(path, {
      path,
      collection,
      title,
      data: parsed.data,
      hasFrontmatter: parsed.hasFrontmatter,
      text,
      body: parsed.body,
      links: extractLinks(text),
      mtime,
      terms,
      length,
    });
  }

  all(collection?: Collection): WikiDoc[] {
    const docs = [...this.docs.values()];
    const filtered = collection ? docs.filter((doc) => doc.collection === collection) : docs;
    return filtered.sort((a, b) => a.path.localeCompare(b.path));
  }

  get(path: string): WikiDoc | undefined {
    return this.docs.get(path);
  }

  /** Find a page by path, path without extension, or file name. */
  find(spec: string): WikiDoc | undefined {
    const path = this.resolveLink(spec);
    return path ? this.docs.get(path) : undefined;
  }

  /** Resolve a wikilink target the way Obsidian does: exact path, then path suffix. */
  resolveLink(target: string): string | null {
    const normalized = normalizePath(target);
    if (!normalized) return null;
    const lookup = this.getLookup();
    const wanted = withMd(normalized).toLowerCase();
    const exact = lookup.byLower.get(wanted) ?? lookup.byLower.get(`${this.layout.wikiDir.toLowerCase()}/${wanted}`);
    if (exact) return exact;
    const candidates = (lookup.byStem.get(fileName(wanted)) ?? []).filter((path) =>
      path.toLowerCase().endsWith(`/${wanted}`),
    );
    if (candidates.length === 0) return null;
    return candidates.reduce((best, path) => (path.length < best.length ? path : best));
  }

  private getLookup() {
    if (!this.lookup) {
      const byLower = new Map<string, string>();
      const byStem = new Map<string, string[]>();
      for (const path of this.docs.keys()) {
        const lower = path.toLowerCase();
        byLower.set(lower, path);
        const name = fileName(lower);
        const list = byStem.get(name);
        if (list) list.push(path);
        else byStem.set(name, [path]);
      }
      this.lookup = { byLower, byStem };
    }
    return this.lookup;
  }

  search(query: string, options: { collections?: Collection[]; limit?: number } = {}): SearchHit[] {
    const collections = options.collections?.length ? options.collections : (["wiki"] as Collection[]);
    const docs = this.all().filter((doc) => collections.includes(doc.collection));
    const terms = [...new Set(tokenize(query))];
    if (docs.length === 0 || terms.length === 0) return [];
    const average = docs.reduce((sum, doc) => sum + doc.length, 0) / docs.length || 1;
    const idf = new Map<string, number>();
    for (const term of terms) {
      const df = docs.filter((doc) => doc.terms.has(term)).length;
      idf.set(term, Math.log(1 + (docs.length - df + 0.5) / (df + 0.5)));
    }
    const hits: SearchHit[] = [];
    for (const doc of docs) {
      let score = 0;
      for (const term of terms) {
        const tf = doc.terms.get(term);
        if (!tf) continue;
        const norm = tf + BM25_K1 * (1 - BM25_B + (BM25_B * doc.length) / average);
        score += ((idf.get(term) as number) * tf * (BM25_K1 + 1)) / norm;
      }
      if (score > 0) hits.push({ path: doc.path, title: doc.title, score, ...snippet(doc.text, terms) });
    }
    hits.sort((a, b) => b.score - a.score || a.path.localeCompare(b.path));
    return hits.slice(0, options.limit ?? 10);
  }

  linkGraph(): LinkGraph {
    const graph: LinkGraph = { outgoing: new Map(), inbound: new Map(), broken: [] };
    for (const doc of this.docs.values()) {
      const out = new Set<string>();
      for (const link of doc.links) {
        if (isAssetTarget(link.target)) continue;
        const resolved = this.resolveLink(link.target);
        if (!resolved) {
          graph.broken.push({ path: doc.path, target: link.target, line: link.line });
        } else if (resolved !== doc.path) {
          out.add(resolved);
          const inbound = graph.inbound.get(resolved) ?? new Set<string>();
          inbound.add(doc.path);
          graph.inbound.set(resolved, inbound);
        }
      }
      graph.outgoing.set(doc.path, out);
    }
    return graph;
  }

  async write(path: string, content: string): Promise<void> {
    const collection = collectionOf(this.layout, path);
    await this.vault.write(path, content);
    if (collection) this.put(path, collection, content, this.now().getTime());
  }

  async remove(path: string): Promise<boolean> {
    const removed = await this.vault.remove(path);
    this.forget(path);
    return removed;
  }

  async readSchema(): Promise<string> {
    return (await this.vault.read(this.layout.schemaPath)) ?? "";
  }

  /** Regenerate `wiki/index.md` from the pages. Writes only when it changed. */
  async rebuildIndexMd(): Promise<string> {
    const content = generateIndexMd(this.all("wiki"), this.layout);
    if (this.docs.get(this.indexPath)?.text !== content) await this.write(this.indexPath, content);
    return content;
  }

  async appendLog(action: string, title: string, details: string[] = []): Promise<void> {
    const current = this.docs.get(this.logPath)?.text ?? (await this.vault.read(this.logPath));
    await this.write(this.logPath, appendLogEntry(current, action, title, details, this.now()));
  }
}

function snippet(text: string, terms: string[]): { line: number; snippet: string } {
  const lines = text.split("\n");
  let best = 0;
  let bestScore = -1;
  lines.forEach((line, index) => {
    if (!line.trim()) return;
    const present = new Set(tokenize(line));
    const score = terms.filter((term) => present.has(term)).length;
    if (score > bestScore) {
      bestScore = score;
      best = index;
    }
  });
  const window = lines.slice(Math.max(0, best - 1), best + 3).join("\n").trim();
  return { line: best + 1, snippet: window.length > 400 ? `${window.slice(0, 400)}…` : window };
}
