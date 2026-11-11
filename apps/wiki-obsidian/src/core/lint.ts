import { asString, asStringList, wikilinkTarget } from "./frontmatter";
import { categoryOf, primarySourceOf } from "./indexMd";
import type { WikiStore } from "./wiki";

export interface LintReport {
  uningestedSources: string[];
  brokenLinks: { path: string; target: string; line: number }[];
  orphanPages: string[];
  missingFrontmatter: string[];
  unlinkedMentions: { path: string; mentions: string; target: string }[];
  healthy: boolean;
}

const MAX_MENTIONS = 50;

/** Deterministic health check. No model call. */
export function lintWiki(store: WikiStore): LintReport {
  const { layout } = store;
  const system = new Set([store.indexPath, store.logPath]);
  const wikiDocs = store.all("wiki").filter((doc) => !system.has(doc.path));
  const graph = store.linkGraph();

  const ingested = new Set<string>();
  for (const doc of wikiDocs) {
    const declared = [primarySourceOf(doc), ...asStringList(doc.data.sources).map((item) => wikilinkTarget(item) ?? item)];
    for (const target of declared) {
      const resolved = target ? store.resolveLink(target) : null;
      if (resolved) ingested.add(resolved);
    }
  }
  const uningestedSources = store
    .all("sources")
    .filter((doc) => {
      const mirror = `${layout.wikiDir}/sources/${doc.path.slice(layout.sourcesDir.length + 1)}`;
      return !ingested.has(doc.path) && !store.get(mirror);
    })
    .map((doc) => doc.path);

  const brokenLinks = graph.broken.filter((link) => link.path.startsWith(`${layout.wikiDir}/`) && !system.has(link.path));

  const orphanPages = wikiDocs
    .filter((doc) => ![...(graph.inbound.get(doc.path) ?? [])].some((from) => !system.has(from)))
    .map((doc) => doc.path);

  const missingFrontmatter = wikiDocs
    .filter((doc) => !asString(doc.data.title) || !asString(doc.data.category))
    .map((doc) => doc.path);

  const unlinkedMentions: LintReport["unlinkedMentions"] = [];
  const named = wikiDocs.filter((doc) => ["entity", "concept"].includes(categoryOf(doc, layout)) && doc.title.length >= 4);
  outer: for (const doc of wikiDocs) {
    const linked = graph.outgoing.get(doc.path);
    const body = doc.body.toLowerCase();
    for (const target of named) {
      if (target.path === doc.path || linked?.has(target.path)) continue;
      const index = body.indexOf(target.title.toLowerCase());
      if (index < 0) continue;
      const before = body[index - 1];
      const after = body[index + target.title.length];
      const boundary = (ch: string | undefined) => ch === undefined || !/[\p{L}\p{N}]/u.test(ch);
      if (!boundary(before) || !boundary(after)) continue;
      unlinkedMentions.push({ path: doc.path, mentions: target.title, target: target.path });
      if (unlinkedMentions.length >= MAX_MENTIONS) break outer;
    }
  }

  return {
    uningestedSources,
    brokenLinks,
    orphanPages,
    missingFrontmatter,
    unlinkedMentions,
    healthy:
      uningestedSources.length + brokenLinks.length + orphanPages.length + missingFrontmatter.length === 0,
  };
}

export function formatLintReport(report: LintReport): string {
  const link = (path: string) => `[[${path.replace(/\.md$/i, "")}]]`;
  const section = (title: string, lines: string[]) =>
    lines.length ? `### ${title} (${lines.length})\n${lines.slice(0, 100).map((line) => `- ${line}`).join("\n")}\n\n` : "";
  if (report.healthy && report.unlinkedMentions.length === 0) return "## Wiki lint\n\nKhông có vấn đề nào.\n";
  return (
    "## Wiki lint\n\n" +
    section("Nguồn chưa ingest", report.uningestedSources.map(link)) +
    section("Link gãy", report.brokenLinks.map((item) => `${link(item.path)} dòng ${item.line}: \`[[${item.target}]]\``)) +
    section("Trang mồ côi", report.orphanPages.map(link)) +
    section("Thiếu frontmatter title/category", report.missingFrontmatter.map(link)) +
    section("Nhắc tới nhưng chưa link", report.unlinkedMentions.map((item) => `${link(item.path)} nhắc “${item.mentions}” → ${link(item.target)}`))
  );
}
