import { describe, expect, it } from "vitest";
import { runAgent } from "./agent";
import { compactDiff, diffLines } from "./diff";
import { parseDoc } from "./frontmatter";
import { lintWiki } from "./lint";
import { parseLogEntries } from "./logMd";
import { normalizePath, scopedPath } from "./paths";
import { WikiSession } from "./session";
import { ToolExecutor } from "./tools";
import { DEFAULT_LAYOUT, type ChatMessage, type LlmClient, type LlmReply, type PendingChange, type VaultPort } from "./types";
import { WikiStore } from "./wiki";

class MemoryVault implements VaultPort {
  files = new Map<string, string>();
  constructor(initial: Record<string, string> = {}) {
    for (const [path, content] of Object.entries(initial)) this.files.set(path, content);
  }
  async list() {
    return [...this.files.keys()];
  }
  async read(path: string) {
    return this.files.get(path) ?? null;
  }
  async write(path: string, content: string) {
    this.files.set(path, content);
  }
  async remove(path: string) {
    return this.files.delete(path);
  }
  mtime() {
    return 0;
  }
}

const NOW = new Date(2026, 10, 10, 9, 30);
const page = (title: string, category: string, body: string) =>
  `---\ntitle: "${title}"\ncategory: "${category}"\n---\n\n# ${title}\n\n${body}\n`;

async function setup(files: Record<string, string>, approve: (change: PendingChange) => Promise<boolean> = async () => true) {
  const vault = new MemoryVault(files);
  const store = new WikiStore(vault, DEFAULT_LAYOUT, () => NOW);
  await store.ensureLoaded();
  const changes: PendingChange[] = [];
  const tools = new ToolExecutor(store, async (change) => {
    changes.push(change);
    return approve(change);
  });
  const call = (name: string, args: Record<string, unknown>) => tools.run(name, JSON.stringify(args));
  return { vault, store, tools, call, changes };
}

/** Replies in order; records what it was sent. */
class ScriptedLlm implements LlmClient {
  requests: { messages: ChatMessage[]; toolNames: string[] }[] = [];
  constructor(private readonly replies: LlmReply[]) {}
  async chat(request: { messages: ChatMessage[]; tools: { name: string }[] }): Promise<LlmReply> {
    this.requests.push({ messages: [...request.messages], toolNames: request.tools.map((tool) => tool.name) });
    const reply = this.replies.shift();
    if (!reply) throw new Error("script exhausted");
    return reply;
  }
}

const toolCall = (name: string, args: Record<string, unknown>, id = "c1"): LlmReply => ({
  content: null,
  toolCalls: [{ id, name, arguments: JSON.stringify(args) }],
});

describe("paths", () => {
  it("rejects paths that could leave the root", () => {
    expect(normalizePath("../secret.md")).toBeNull();
    expect(normalizePath("wiki/../../x.md")).toBeNull();
    expect(normalizePath("wiki//x.md")).toBeNull();
    expect(normalizePath("/wiki\\entities\\a.md")).toBe("wiki/entities/a.md");
  });

  it("accepts a tool path with or without the collection folder", () => {
    expect(scopedPath(DEFAULT_LAYOUT, "wiki", "entities/rust")).toBe("wiki/entities/rust.md");
    expect(scopedPath(DEFAULT_LAYOUT, "wiki", "wiki/entities/rust.md")).toBe("wiki/entities/rust.md");
    expect(scopedPath(DEFAULT_LAYOUT, "sources", "../wiki/x.md")).toBeNull();
  });
});

describe("frontmatter", () => {
  it("reads scalars, inline arrays, block lists and skips nested maps", () => {
    const doc = parseDoc(
      '---\ntitle: "A \\"quoted\\" title"\nsource_count: 2\ntags: [a, "b c"]\nsources:\n  - "[[sources/x.md|x.md]]"\n  - "[[sources/y.md|y.md]]"\nqmd:\n  metadata:\n    category: "source_summary"\n---\nBody\n',
    );
    expect(doc.data).toEqual({
      title: 'A "quoted" title',
      source_count: 2,
      tags: ["a", "b c"],
      sources: ["[[sources/x.md|x.md]]", "[[sources/y.md|y.md]]"],
    });
    expect(doc.body).toBe("Body\n");
  });
});

describe("store", () => {
  it("searches without diacritics and ranks the title match first", async () => {
    const { store } = await setup({
      "wiki/concepts/dca.md": page("Chiến lược DCA", "concept", "Trung bình giá theo thời gian."),
      "wiki/concepts/grid.md": page("Grid", "concept", "Khác với chiến lược DCA ở khoảng cách lệnh."),
      "sources/raw.md": "chien luoc DCA raw",
    });
    const hits = store.search("chien luoc dca");
    expect(hits.map((hit) => hit.path)).toEqual(["wiki/concepts/dca.md", "wiki/concepts/grid.md"]);
    expect(store.search("chien luoc", { collections: ["sources"] }).map((hit) => hit.path)).toEqual(["sources/raw.md"]);
  });

  it("resolves wikilinks by exact path, wiki-relative path and file name", async () => {
    const { store } = await setup({
      "wiki/entities/rust.md": page("Rust", "entity", "x"),
      "sources/docs/lib.md": "raw",
    });
    expect(store.resolveLink("wiki/entities/rust")).toBe("wiki/entities/rust.md");
    expect(store.resolveLink("entities/rust")).toBe("wiki/entities/rust.md");
    expect(store.resolveLink("Rust")).toBe("wiki/entities/rust.md");
    expect(store.resolveLink("sources/docs/lib.md")).toBe("sources/docs/lib.md");
    expect(store.resolveLink("missing")).toBeNull();
  });

  it("ignores links inside code fences", async () => {
    const { store } = await setup({ "wiki/a.md": "[[b]]\n```\n[[ghost]]\n```\n", "wiki/b.md": "b" });
    expect(store.linkGraph().broken).toEqual([]);
    expect([...(store.linkGraph().inbound.get("wiki/b.md") ?? [])]).toEqual(["wiki/a.md"]);
  });
});

describe("tools", () => {
  it("writes a page after approval and refreshes index.md", async () => {
    const { call, vault, changes } = await setup({});
    const result = await call("wiki_write_file", { rel_path: "entities/rust", content: page("Rust", "entity", "A language.") });
    expect(result).toEqual({ ok: true, content: "Created wiki/entities/rust.md." });
    expect(changes).toHaveLength(1);
    expect(changes[0]).toMatchObject({ path: "wiki/entities/rust.md", before: null });
    expect(vault.files.get("wiki/index.md")).toContain("- [[wiki/entities/rust|Rust]] — A language.");
  });

  it("leaves the vault untouched when the user declines", async () => {
    const { call, vault } = await setup({}, async () => false);
    const result = await call("wiki_write_file", { rel_path: "entities/rust.md", content: "x" });
    expect(result.ok).toBe(false);
    expect(result.content).toContain("declined");
    expect(vault.files.size).toBe(0);
  });

  it("refuses paths outside the collections and direct index writes", async () => {
    const { call, vault } = await setup({});
    expect((await call("wiki_write_file", { rel_path: "../../etc/passwd", content: "x" })).ok).toBe(false);
    expect((await call("wiki_write_file", { rel_path: "index.md", content: "x" })).ok).toBe(false);
    expect(vault.files.size).toBe(0);
  });

  it("patches only a unique target text", async () => {
    const { call, vault } = await setup({ "wiki/a.md": "one two one\n" });
    expect((await call("wiki_patch_file", { rel_path: "a.md", target_text: "one", replacement_text: "1" })).ok).toBe(false);
    expect((await call("wiki_patch_file", { rel_path: "a.md", target_text: "two", replacement_text: "2", append_text: "tail" })).ok).toBe(true);
    expect(vault.files.get("wiki/a.md")).toBe("one 2 one\ntail\n");
  });

  it("ingests a source into a same-named summary page, the index and the log", async () => {
    const { call, vault } = await setup({ "sources/algo/dca.md": "raw text" });
    const result = await call("wiki_ingest_source", {
      source_rel_path: "sources/algo/dca.md",
      title: "DCA Bot",
      one_line_summary: "Averaging down",
      wiki_markdown_body: "Uses [[entities/mt5|MT5]].",
      tags: ["#dca"],
      related_entities: ["mt5"],
      related_concepts: ["martingale"],
    });
    expect(result.ok).toBe(true);
    const summary = vault.files.get("wiki/sources/algo/dca.md") ?? "";
    expect(summary).toContain('source_file: "[[sources/algo/dca.md|dca.md]]"');
    expect(summary).toContain('  - "dca"');
    expect(summary).toContain("- **Concepts:** [[concepts/martingale|martingale]]");
    expect(summary).not.toContain("**Entities:**");
    expect(vault.files.get("wiki/index.md")).toContain("primary: [[sources/algo/dca.md|dca.md]]");
    expect(parseLogEntries(vault.files.get("wiki/log.md") ?? "", 1)[0]).toMatchObject({
      timestamp: "2026-11-10 09:30",
      action: "ingest",
      title: "DCA Bot",
    });
  });

  it("reads a line range with line numbers and the schema file", async () => {
    const { call } = await setup({ "wiki/a.md": "l1\nl2\nl3\nl4", "SCHEMA.md": "rules" });
    expect((await call("wiki_get", { file: "wiki/a.md:2:2" })).content).toBe("# wiki/a.md (lines 2-3 of 4)\n2: l2\n3: l3");
    expect((await call("wiki_get", { file: "SCHEMA.md", line_numbers: false })).content).toContain("rules");
    expect((await call("wiki_get", { file: "nope" })).ok).toBe(false);
  });

  it("offers no write tools in read-only mode", async () => {
    const { store } = await setup({});
    const readOnly = new ToolExecutor(store, async () => true, { readOnly: true });
    expect((await readOnly.run("wiki_write_file", '{"rel_path":"a.md","content":"x"}')).ok).toBe(false);
    expect((await readOnly.run("wiki_tree", '{"scope":"all"}')).ok).toBe(true);
  });

  it("reports malformed arguments instead of throwing", async () => {
    const { tools } = await setup({});
    expect((await tools.run("wiki_get", "{not json")).ok).toBe(false);
    expect((await tools.run("wiki_get", "{}")).content).toContain("`file`");
  });
});

describe("lint", () => {
  it("finds pending sources, broken links, orphans and missing frontmatter", async () => {
    const { store } = await setup({
      "sources/done.md": "raw",
      "sources/pending.md": "raw",
      "wiki/sources/done.md": page("Done", "source_summary", "See [[entities/rust|Rust]] and [[ghost]]."),
      "wiki/entities/rust.md": page("Rust", "entity", "Linked from [[wiki/sources/done]]."),
      "wiki/concepts/lonely.md": "No frontmatter, mentions Rust here.",
    });
    const report = lintWiki(store);
    expect(report.uningestedSources).toEqual(["sources/pending.md"]);
    expect(report.brokenLinks).toEqual([{ path: "wiki/sources/done.md", target: "ghost", line: 8 }]);
    expect(report.orphanPages).toEqual(["wiki/concepts/lonely.md"]);
    expect(report.missingFrontmatter).toEqual(["wiki/concepts/lonely.md"]);
    expect(report.unlinkedMentions).toEqual([{ path: "wiki/concepts/lonely.md", mentions: "Rust", target: "wiki/entities/rust.md" }]);
    expect(report.healthy).toBe(false);
  });
});

describe("agent", () => {
  it("runs tool rounds, then returns the answer", async () => {
    const { tools } = await setup({ "wiki/a.md": "alpha" });
    const llm = new ScriptedLlm([toolCall("wiki_get", { file: "wiki/a.md" }), { content: "It says alpha.", toolCalls: [] }]);
    const outcome = await runAgent({
      llm,
      tools: [],
      runTool: (name, args) => tools.run(name, args),
      messages: [{ role: "user", content: "what?" }],
      maxSteps: 3,
    });
    expect(outcome.answer).toBe("It says alpha.");
    expect(outcome.toolCalls).toBe(1);
    expect(outcome.messages.map((message) => message.role)).toEqual(["user", "assistant", "tool", "assistant"]);
    expect(outcome.messages[2]?.content).toContain("1: alpha");
  });

  it("withholds tools on the round after the step limit", async () => {
    const llm = new ScriptedLlm([toolCall("wiki_tree", { scope: "all" }), { content: "done", toolCalls: [] }]);
    const outcome = await runAgent({
      llm,
      tools: [{ name: "wiki_tree", description: "", parameters: {} }],
      runTool: async () => ({ ok: true, content: "ok" }),
      messages: [{ role: "user", content: "go" }],
      maxSteps: 1,
    });
    expect(llm.requests.map((request) => request.toolNames)).toEqual([["wiki_tree"], []]);
    expect(outcome.hitStepLimit).toBe(true);
  });

  it("stops when aborted", async () => {
    const controller = new AbortController();
    controller.abort();
    await expect(
      runAgent({ llm: new ScriptedLlm([]), tools: [], runTool: async () => ({ ok: true, content: "" }), messages: [], maxSteps: 1, signal: controller.signal }),
    ).rejects.toThrow("Đã dừng");
  });
});

describe("session", () => {
  it("gives ask mode read tools only and keeps just questions and answers between turns", async () => {
    const { store } = await setup({ "wiki/a.md": "alpha", "SCHEMA.md": "house rules" });
    const llm = new ScriptedLlm([
      toolCall("wiki_get", { file: "wiki/a.md" }),
      { content: "alpha", toolCalls: [] },
      { content: "second", toolCalls: [] },
    ]);
    const session = new WikiSession(store, llm, async () => true, { language: "auto", maxSteps: 4, ingestMaxSteps: 8 });
    await session.send("Trang a nói gì vậy ạ?", "ask");
    await session.send("again", "agent");

    expect(llm.requests[0]?.toolNames).not.toContain("wiki_write_file");
    expect(llm.requests[0]?.messages[0]?.content).toContain("MANDATORY OUTPUT LANGUAGE: Vietnamese");
    expect(llm.requests[0]?.messages[0]?.content).toContain("house rules");
    expect(llm.requests[2]?.toolNames).toContain("wiki_write_file");
    expect(llm.requests[2]?.messages.map((message) => message.role)).toEqual(["system", "user", "assistant", "user"]);
  });

  it("ingest sends the source text and the ingest workflow", async () => {
    const { store } = await setup({ "sources/n.md": "source body" });
    const llm = new ScriptedLlm([{ content: "ok", toolCalls: [] }]);
    const session = new WikiSession(store, llm, async () => true, { language: "English", maxSteps: 4, ingestMaxSteps: 8 });
    await session.ingest("sources/n.md");
    const [system, user] = llm.requests[0]?.messages ?? [];
    expect(system?.content).toContain('`source_rel_path`: `"n.md"`');
    expect(user?.content).toContain("source body");
    await expect(session.ingest("wiki/x.md")).rejects.toThrow();
  });
});

describe("diff", () => {
  it("marks changed lines and collapses unchanged runs", () => {
    const lines = diffLines("a\nb\nc\nd\ne\nf\ng", "a\nb\nc\nd\nX\nf\ng") ?? [];
    expect(lines.filter((line) => line.kind !== "same")).toEqual([
      { kind: "del", text: "e" },
      { kind: "add", text: "X" },
    ]);
    const compact = compactDiff(lines, 1);
    expect(compact.map((line) => (line ? line.text : null))).toEqual(["d", "e", "X", "f"]);
  });
});
