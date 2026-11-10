import type { ToolResult } from "./agent";
import { asStringList, wikilinkTarget } from "./frontmatter";
import { formatLintReport, lintWiki } from "./lint";
import { parseLogEntries } from "./logMd";
import { collectionOf, normalizePath, scopedPath, sourceInnerPath } from "./paths";
import { renderSourceSummary, renderSynthesis, type IngestRequest, type SynthesisRequest } from "./render";
import type { ApproveChange, Collection, ToolSpec } from "./types";
import type { WikiStore } from "./wiki";

const scopeParam = {
  type: "string",
  enum: ["wiki", "sources"],
  default: "wiki",
  description: "Target collection folder. Defaults to 'wiki'.",
};

/** Tool names and shapes follow llm-wiki-rs so its prompts carry over. */
export const TOOL_SPECS: ToolSpec[] = [
  {
    name: "wiki_tree",
    description: "List the Markdown files of `sources`, `wiki`, or both (`all`), with each page's title.",
    parameters: {
      type: "object",
      required: ["scope"],
      properties: {
        scope: { type: "string", enum: ["all", "sources", "wiki"] },
        sub_path: { type: "string", description: "Optional subfolder inside the scope, e.g. 'entities'." },
      },
    },
  },
  {
    name: "wiki_query",
    description:
      "Full-text search (BM25, diacritic-insensitive) across `wiki` (default) or `sources`. Returns ranked pages with a snippet and its line number.",
    parameters: {
      type: "object",
      required: ["query"],
      properties: {
        query: { type: "string", description: "Keywords. Try translations and variants when results are thin." },
        collections: { type: "array", items: { type: "string", enum: ["wiki", "sources"] }, default: ["wiki"] },
        limit: { type: "integer", default: 10 },
      },
    },
  },
  {
    name: "wiki_get",
    description:
      "Read one document by path (e.g. 'wiki/entities/rust.md', 'sources/docs/lib.md', 'SCHEMA.md') or file name. Supports a `path:from:count` suffix or `from_line`/`max_lines`.",
    parameters: {
      type: "object",
      required: ["file"],
      properties: {
        file: { type: "string" },
        from_line: { type: "integer", description: "1-indexed start line." },
        max_lines: { type: "integer" },
        line_numbers: { type: "boolean", default: true },
      },
    },
  },
  {
    name: "wiki_multi_get",
    description: "Read several documents by glob (e.g. 'wiki/entities/*.md') or a comma-separated list of paths.",
    parameters: {
      type: "object",
      required: ["pattern"],
      properties: {
        pattern: { type: "string" },
        max_lines: { type: "integer", description: "Maximum lines per file." },
      },
    },
  },
  {
    name: "wiki_graph_neighbors",
    description: "Backlinks and outgoing [[WikiLinks]] of one document, or a graph summary when `rel_path` is omitted.",
    parameters: { type: "object", properties: { rel_path: { type: "string" } } },
  },
  {
    name: "wiki_write_file",
    description:
      "Create or overwrite a Markdown file inside `wiki` or `sources`. Parent folders are created. `wiki/index.md` refreshes automatically.",
    parameters: {
      type: "object",
      required: ["rel_path", "content"],
      properties: {
        scope: scopeParam,
        rel_path: { type: "string", description: "Path inside the collection, e.g. 'entities/rust.md'." },
        content: { type: "string", description: "Full Markdown with YAML frontmatter and [[WikiLinks]]." },
      },
    },
  },
  {
    name: "wiki_patch_file",
    description:
      "Edit an existing Markdown file: replace one exact text block (`target_text` → `replacement_text`) and/or append `append_text`.",
    parameters: {
      type: "object",
      required: ["rel_path"],
      properties: {
        scope: scopeParam,
        rel_path: { type: "string" },
        target_text: { type: "string", description: "Exact text to replace. Must occur exactly once." },
        replacement_text: { type: "string" },
        append_text: { type: "string" },
      },
    },
  },
  {
    name: "wiki_ingest_source",
    description:
      "Write the wiki summary page for a raw source. Keeps the exact source file name ('sources/a/b.md' → 'wiki/sources/a/b.md'), links it to the source, refreshes `wiki/index.md` and logs to `wiki/log.md`.",
    parameters: {
      type: "object",
      required: ["source_rel_path", "title", "one_line_summary", "wiki_markdown_body"],
      properties: {
        source_rel_path: { type: "string", description: "Path of the source inside `sources/`." },
        target_wiki_path: { type: "string", description: "Optional existing wiki page to update instead." },
        raw_source_content: { type: "string", description: "Optional: create the raw source file first." },
        title: { type: "string" },
        one_line_summary: { type: "string" },
        wiki_markdown_body: { type: "string", description: "Synthesized Markdown body with [[WikiLinks]]." },
        tags: { type: "array", items: { type: "string" } },
        related_entities: { type: "array", items: { type: "string" } },
        related_concepts: { type: "array", items: { type: "string" } },
      },
    },
  },
  {
    name: "wiki_file_synthesis",
    description:
      "File a valuable answer, comparison or analysis into `wiki/syntheses/` as a permanent page, refreshing `wiki/index.md` and logging a `query` entry.",
    parameters: {
      type: "object",
      required: ["rel_path", "query", "title", "one_line_summary", "markdown_body"],
      properties: {
        rel_path: { type: "string", description: "File name inside `wiki/syntheses/`, e.g. 'rag-vs-llm-wiki.md'." },
        query: { type: "string" },
        title: { type: "string" },
        one_line_summary: { type: "string" },
        markdown_body: { type: "string" },
        tags: { type: "array", items: { type: "string" } },
      },
    },
  },
  {
    name: "wiki_lint",
    description:
      "Health-check the wiki: un-ingested sources, broken [[WikiLinks]], orphan pages, missing frontmatter, unlinked mentions.",
    parameters: { type: "object", properties: {} },
  },
  {
    name: "wiki_log",
    description: "Read recent entries of `wiki/log.md`, or append one (`## [YYYY-MM-DD HH:MM] action | title`).",
    parameters: {
      type: "object",
      properties: {
        mode: { type: "string", enum: ["read", "append"], default: "read" },
        limit: { type: "integer", default: 10 },
        action: { type: "string", description: "'ingest', 'query', 'lint' or 'update' when appending." },
        title: { type: "string" },
        details: { type: "array", items: { type: "string" } },
      },
    },
  },
  {
    name: "wiki_schema",
    description: "Read or replace `SCHEMA.md`, the document defining this wiki's conventions and workflows.",
    parameters: {
      type: "object",
      properties: {
        mode: { type: "string", enum: ["read", "write"], default: "read" },
        content: { type: "string" },
      },
    },
  },
  {
    name: "wiki_delete_file",
    description: "Delete an obsolete or duplicate Markdown page inside `wiki` or `sources`.",
    parameters: {
      type: "object",
      required: ["rel_path"],
      properties: { scope: scopeParam, rel_path: { type: "string" } },
    },
  },
];

export const READ_ONLY_TOOLS = ["wiki_tree", "wiki_query", "wiki_get", "wiki_multi_get", "wiki_graph_neighbors"];

export function toolSpecs(readOnly: boolean): ToolSpec[] {
  return readOnly ? TOOL_SPECS.filter((spec) => READ_ONLY_TOOLS.includes(spec.name)) : TOOL_SPECS;
}

type Args = Record<string, unknown>;

class ToolError extends Error {}

function text(args: Args, key: string): string {
  const value = args[key];
  if (typeof value !== "string" || !value.trim()) throw new ToolError(`Missing required string argument \`${key}\`.`);
  return value;
}

function optionalText(args: Args, key: string): string | undefined {
  const value = args[key];
  return typeof value === "string" && value !== "" ? value : undefined;
}

function optionalInt(args: Args, key: string): number | undefined {
  const value = typeof args[key] === "string" ? Number(args[key]) : args[key];
  return typeof value === "number" && Number.isFinite(value) ? Math.trunc(value) : undefined;
}

function strings(args: Args, key: string): string[] {
  const value = args[key];
  return Array.isArray(value) ? value.filter((item): item is string => typeof item === "string") : [];
}

function scope(args: Args): Collection {
  return args.scope === "sources" ? "sources" : "wiki";
}

function numbered(lines: string[], first: number): string {
  return lines.map((line, index) => `${first + index}: ${line}`).join("\n");
}

function globToRegExp(glob: string): RegExp {
  const escaped = glob.replace(/[.+^${}()|[\]\\]/g, "\\$&").replace(/\*\*\/?/g, "\u0000").replace(/\*/g, "[^/]*").replace(/\?/g, "[^/]").replace(/\u0000/g, ".*");
  return new RegExp(`^${escaped}$`, "i");
}

/** Runs tool calls against the store. Every file change goes through `approve` first. */
export class ToolExecutor {
  constructor(
    private readonly store: WikiStore,
    private readonly approve: ApproveChange,
    private readonly options: { readOnly?: boolean; maxChars?: number } = {},
  ) {}

  private get maxChars(): number {
    return this.options.maxChars ?? 12_000;
  }

  async run(name: string, rawArguments: string): Promise<ToolResult> {
    if (!toolSpecs(this.options.readOnly ?? false).some((spec) => spec.name === name)) {
      return { ok: false, content: `Unknown or unavailable tool \`${name}\`.` };
    }
    let args: Args;
    try {
      const parsed: unknown = rawArguments.trim() ? JSON.parse(rawArguments) : {};
      if (typeof parsed !== "object" || parsed === null || Array.isArray(parsed)) throw new Error("not an object");
      args = parsed as Args;
    } catch {
      return { ok: false, content: "Arguments were not a valid JSON object. Fix them and call the tool again." };
    }
    try {
      await this.store.ensureLoaded();
      return { ok: true, content: this.bound(await this.dispatch(name, args)) };
    } catch (error) {
      if (error instanceof ToolError) return { ok: false, content: error.message };
      throw error;
    }
  }

  private bound(content: string): string {
    if (content.length <= this.maxChars) return content;
    return `${content.slice(0, this.maxChars)}\n\n[Truncated at ${this.maxChars} characters. Read a narrower range with from_line/max_lines.]`;
  }

  private dispatch(name: string, args: Args): Promise<string> | string {
    switch (name) {
      case "wiki_tree":
        return this.tree(args);
      case "wiki_query":
        return this.query(args);
      case "wiki_get":
        return this.get(args);
      case "wiki_multi_get":
        return this.multiGet(args);
      case "wiki_graph_neighbors":
        return this.neighbors(args);
      case "wiki_write_file":
        return this.writeFile(args);
      case "wiki_patch_file":
        return this.patchFile(args);
      case "wiki_ingest_source":
        return this.ingest(args);
      case "wiki_file_synthesis":
        return this.synthesis(args);
      case "wiki_lint":
        return formatLintReport(lintWiki(this.store));
      case "wiki_log":
        return this.log(args);
      case "wiki_schema":
        return this.schema(args);
      case "wiki_delete_file":
        return this.deleteFile(args);
      default:
        throw new ToolError(`Unknown tool \`${name}\`.`);
    }
  }

  // ----- reads -----

  private tree(args: Args): string {
    const { layout } = this.store;
    const which = args.scope === "sources" || args.scope === "wiki" ? (args.scope as Collection) : undefined;
    const sub = optionalText(args, "sub_path");
    const subPath = sub ? normalizePath(sub) : null;
    const docs = this.store.all(which).filter((doc) => {
      if (!subPath) return true;
      const dir = doc.collection === "wiki" ? layout.wikiDir : layout.sourcesDir;
      return doc.path.startsWith(`${dir}/${subPath}/`) || doc.path.startsWith(`${subPath}/`);
    });
    if (docs.length === 0) return "No Markdown files found.";
    return `${docs.length} files\n${docs.map((doc) => `${doc.path} — ${doc.title}`).join("\n")}`;
  }

  private query(args: Args): string {
    const collections = strings(args, "collections").filter(
      (item): item is Collection => item === "wiki" || item === "sources",
    );
    const limit = Math.min(Math.max(optionalInt(args, "limit") ?? 10, 1), 30);
    const hits = this.store.search(text(args, "query"), { collections, limit });
    if (hits.length === 0) return "No results. Try other keywords, a translation, or `collections: [\"wiki\", \"sources\"]`.";
    return hits
      .map((hit, index) => `${index + 1}. ${hit.path} — ${hit.title} (score ${hit.score.toFixed(2)}, line ${hit.line})\n${hit.snippet}`)
      .join("\n\n");
  }

  private async readable(spec: string): Promise<{ path: string; content: string }> {
    const { layout } = this.store;
    const normalized = normalizePath(spec);
    if (!normalized) throw new ToolError(`Invalid path \`${spec}\`.`);
    if (normalized.toLowerCase() === layout.schemaPath.toLowerCase()) {
      return { path: layout.schemaPath, content: await this.store.readSchema() };
    }
    const doc = this.store.find(normalized);
    if (!doc) throw new ToolError(`No document matches \`${spec}\`. Use wiki_tree or wiki_query to find the path.`);
    return { path: doc.path, content: doc.text };
  }

  private async get(args: Args): Promise<string> {
    let file = text(args, "file").trim();
    let from = optionalInt(args, "from_line");
    let count = optionalInt(args, "max_lines");
    const suffix = /^(.*?):(\d+)(?::(\d+))?$/.exec(file);
    if (suffix) {
      file = suffix[1] as string;
      from ??= Number(suffix[2]);
      if (suffix[3]) count ??= Number(suffix[3]);
    }
    const { path, content } = await this.readable(file);
    const lines = content.split("\n");
    const first = Math.max(from ?? 1, 1);
    const slice = lines.slice(first - 1, count ? first - 1 + count : undefined);
    const shown = args.line_numbers === false ? slice.join("\n") : numbered(slice, first);
    return `# ${path} (lines ${first}-${first + slice.length - 1} of ${lines.length})\n${shown}`;
  }

  private async multiGet(args: Args): Promise<string> {
    const pattern = text(args, "pattern");
    const maxLines = optionalInt(args, "max_lines");
    let paths: string[];
    if (/[*?]/.test(pattern)) {
      const matcher = globToRegExp(normalizePath(pattern) ?? pattern);
      paths = this.store.all().map((doc) => doc.path).filter((path) => matcher.test(path));
    } else {
      paths = pattern.split(",").map((item) => item.trim()).filter(Boolean);
    }
    if (paths.length === 0) return "No documents matched.";
    const parts: string[] = [];
    for (const spec of paths.slice(0, 20)) {
      try {
        const { path, content } = await this.readable(spec);
        const lines = content.split("\n");
        parts.push(`# ${path}\n${numbered(maxLines ? lines.slice(0, maxLines) : lines, 1)}`);
      } catch (error) {
        parts.push(`# ${spec}\n${error instanceof Error ? error.message : String(error)}`);
      }
    }
    if (paths.length > 20) parts.push(`[${paths.length - 20} more files not shown]`);
    return parts.join("\n\n");
  }

  private neighbors(args: Args): string {
    const graph = this.store.linkGraph();
    const spec = optionalText(args, "rel_path");
    if (!spec) {
      const ranked = [...graph.inbound.entries()].sort((a, b) => b[1].size - a[1].size).slice(0, 15);
      return JSON.stringify({
        documents: graph.outgoing.size,
        links: [...graph.outgoing.values()].reduce((sum, set) => sum + set.size, 0),
        broken_links: graph.broken.length,
        most_linked: ranked.map(([path, from]) => ({ path, backlinks: from.size })),
      });
    }
    const doc = this.store.find(spec);
    if (!doc) throw new ToolError(`No document matches \`${spec}\`.`);
    return JSON.stringify({
      path: doc.path,
      outgoing_links: [...(graph.outgoing.get(doc.path) ?? [])].sort(),
      backlinks: [...(graph.inbound.get(doc.path) ?? [])].sort(),
      unresolved_links: graph.broken.filter((link) => link.path === doc.path).map((link) => link.target),
    });
  }

  // ----- writes -----

  private target(args: Args, key = "rel_path", collection: Collection = scope(args)): string {
    const path = scopedPath(this.store.layout, collection, text(args, key));
    if (!path) throw new ToolError(`Invalid path \`${String(args[key])}\`.`);
    if (path === this.store.indexPath) throw new ToolError("`wiki/index.md` is generated automatically. Edit the pages instead.");
    return path;
  }

  /** Ask the user, then write or delete. Throws ToolError when declined. */
  private async apply(path: string, after: string | null): Promise<void> {
    const before = this.store.get(path)?.text ?? (await this.store.vault.read(path));
    if (before === after) return;
    if (!(await this.approve({ path, before, after }))) {
      throw new ToolError(`The user declined the change to \`${path}\`. Do not retry it; ask what they want instead.`);
    }
    if (after === null) await this.store.remove(path);
    else await this.store.write(path, after);
    if (collectionOf(this.store.layout, path) === "wiki" && path !== this.store.logPath) {
      await this.store.rebuildIndexMd();
    }
  }

  private async writeFile(args: Args): Promise<string> {
    const path = this.target(args);
    const existed = this.store.get(path) !== undefined;
    await this.apply(path, text(args, "content"));
    return `${existed ? "Updated" : "Created"} ${path}.`;
  }

  private async patchFile(args: Args): Promise<string> {
    const path = this.target(args);
    const doc = this.store.get(path);
    if (!doc) throw new ToolError(`\`${path}\` does not exist. Create it with wiki_write_file.`);
    const targetText = optionalText(args, "target_text");
    const appendText = optionalText(args, "append_text");
    if (!targetText && !appendText) throw new ToolError("Provide `target_text` with `replacement_text`, or `append_text`.");
    let next = doc.text;
    if (targetText) {
      const first = next.indexOf(targetText);
      if (first < 0) throw new ToolError("`target_text` was not found. Read the file again and copy the text exactly.");
      if (next.indexOf(targetText, first + 1) >= 0) {
        throw new ToolError("`target_text` occurs more than once. Include more surrounding text so it is unique.");
      }
      const replacement = typeof args.replacement_text === "string" ? args.replacement_text : "";
      next = next.slice(0, first) + replacement + next.slice(first + targetText.length);
    }
    if (appendText) next = `${next.endsWith("\n") ? next : `${next}\n`}${appendText}${appendText.endsWith("\n") ? "" : "\n"}`;
    await this.apply(path, next);
    return `Patched ${path}.`;
  }

  private async ingest(args: Args): Promise<string> {
    const { layout } = this.store;
    const inner = sourceInnerPath(layout, text(args, "source_rel_path"));
    if (!inner) throw new ToolError("Invalid `source_rel_path`.");
    const sourcePath = `${layout.sourcesDir}/${inner}`;
    const rawSource = optionalText(args, "raw_source_content");
    if (!this.store.get(sourcePath)) {
      if (!rawSource) throw new ToolError(`Source \`${sourcePath}\` does not exist. Pass \`raw_source_content\` to create it.`);
      await this.apply(sourcePath, rawSource);
    }
    const wikiPath = optionalText(args, "target_wiki_path")
      ? this.target(args, "target_wiki_path", "wiki")
      : `${layout.wikiDir}/sources/${inner}`;
    const request: IngestRequest = {
      source_rel_path: inner,
      title: text(args, "title"),
      one_line_summary: text(args, "one_line_summary"),
      wiki_markdown_body: text(args, "wiki_markdown_body"),
      tags: strings(args, "tags"),
      related_entities: strings(args, "related_entities"),
      related_concepts: strings(args, "related_concepts"),
    };
    const previous = asStringList(this.store.get(wikiPath)?.data.sources).map((item) => wikilinkTarget(item) ?? item);
    await this.apply(wikiPath, renderSourceSummary(request, sourcePath, previous, this.store.now()));
    await this.store.appendLog("ingest", request.title, [`source: ${sourcePath}`, `wiki: ${wikiPath}`]);
    return `Ingested ${sourcePath} → ${wikiPath}. Now create or update the related entity and concept pages.`;
  }

  private async synthesis(args: Args): Promise<string> {
    const { layout } = this.store;
    const raw = normalizePath(text(args, "rel_path"));
    if (!raw) throw new ToolError("Invalid `rel_path`.");
    const inner = raw.replace(new RegExp(`^(${layout.wikiDir}/)?syntheses/`), "");
    const path = scopedPath(layout, "wiki", `syntheses/${inner}`);
    if (!path) throw new ToolError("Invalid `rel_path`.");
    const request: SynthesisRequest = {
      rel_path: inner,
      query: text(args, "query"),
      title: text(args, "title"),
      one_line_summary: text(args, "one_line_summary"),
      markdown_body: text(args, "markdown_body"),
      tags: strings(args, "tags"),
    };
    await this.apply(path, renderSynthesis(request, this.store.now()));
    await this.store.appendLog("query", request.title, [`question: ${request.query}`, `wiki: ${path}`]);
    return `Filed synthesis at ${path}.`;
  }

  private async log(args: Args): Promise<string> {
    if (args.mode === "append") {
      await this.store.appendLog(optionalText(args, "action") ?? "update", text(args, "title"), strings(args, "details"));
      return "Log entry appended.";
    }
    const log = this.store.get(this.store.logPath)?.text ?? "";
    const entries = parseLogEntries(log, Math.max(optionalInt(args, "limit") ?? 10, 1));
    return entries.length ? JSON.stringify(entries) : "The log is empty.";
  }

  private async schema(args: Args): Promise<string> {
    if (args.mode !== "write") return (await this.store.readSchema()) || "SCHEMA.md is empty.";
    await this.apply(this.store.layout.schemaPath, text(args, "content"));
    return "SCHEMA.md updated.";
  }

  private async deleteFile(args: Args): Promise<string> {
    const path = this.target(args);
    if (path === this.store.logPath) throw new ToolError("`wiki/log.md` cannot be deleted.");
    if (!this.store.get(path)) throw new ToolError(`\`${path}\` does not exist.`);
    await this.apply(path, null);
    return `Deleted ${path}.`;
  }
}
