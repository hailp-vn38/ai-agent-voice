import { asString, asStringList, wikilinkTarget } from "./frontmatter";
import { fileName, stripMd } from "./paths";
import type { Layout } from "./types";
import type { WikiDoc } from "./wiki";

const HEADER =
  '---\ntitle: "Wiki Master Index"\ncategory: "system"\ntags:\n  - "wiki/system"\n  - "wiki/index"\n---\n\n# Wiki Knowledge Base Index\n\n> Content-oriented catalog of everything in the wiki.\n\n';

type Category = "source_summary" | "entity" | "concept" | "synthesis" | "other";

export function categoryOf(doc: WikiDoc, layout: Layout): Category {
  const declared = asString(doc.data.category);
  if (declared === "source_summary" || declared === "source") return "source_summary";
  if (declared === "entity" || declared === "concept" || declared === "synthesis") return declared;
  if (declared) return "other";
  const inner = doc.path.slice(layout.wikiDir.length + 1);
  if (inner.startsWith("sources/")) return "source_summary";
  if (inner.startsWith("entities/")) return "entity";
  if (inner.startsWith("concepts/")) return "concept";
  if (inner.startsWith("syntheses/")) return "synthesis";
  return "other";
}

/** The raw source a summary page was compiled from, if it declares one. */
export function primarySourceOf(doc: WikiDoc): string | null {
  const declared = asString(doc.data.source_file);
  return declared ? (wikilinkTarget(declared) ?? declared) : null;
}

function summaryOf(doc: WikiDoc): string {
  const declared = asString(doc.data.summary);
  if (declared) return declared;
  for (const raw of doc.body.split("\n")) {
    const line = raw.trim();
    if (line && !/^(#|>|```|---)/.test(line)) return [...line].slice(0, 120).join("");
  }
  return doc.title;
}

function updatedOf(doc: WikiDoc): string {
  const declared = asString(doc.data.updated_at);
  if (declared && declared.length >= 10) return declared.slice(0, 10);
  return new Date(doc.mtime).toISOString().slice(0, 10);
}

function entryLine(doc: WikiDoc): string {
  const target = stripMd(doc.path);
  const source = primarySourceOf(doc);
  if (source) {
    const declaredCount = typeof doc.data.source_count === "number" ? doc.data.source_count : 0;
    const count = Math.max(1, declaredCount, asStringList(doc.data.sources).length);
    const name = fileName(doc.path);
    return `- [[${target}|${name}]] — ${summaryOf(doc)} *(sources: ${count}, updated: ${updatedOf(doc)}, primary: [[${source}|${name}]])*\n`;
  }
  return `- [[${target}|${doc.title}]] — ${summaryOf(doc)} *(updated: ${updatedOf(doc)})*\n`;
}

/** Build `wiki/index.md`: every wiki page, grouped by category. */
export function generateIndexMd(wikiDocs: WikiDoc[], layout: Layout): string {
  const groups: Record<Category, WikiDoc[]> = {
    source_summary: [],
    entity: [],
    concept: [],
    synthesis: [],
    other: [],
  };
  const skip = new Set([`${layout.wikiDir}/index.md`, `${layout.wikiDir}/log.md`]);
  for (const doc of wikiDocs) {
    if (!skip.has(doc.path)) groups[categoryOf(doc, layout)].push(doc);
  }
  const section = (heading: string, docs: WikiDoc[], empty: string) =>
    `${heading}\n${docs.length ? docs.map(entryLine).join("") : `${empty}\n`}\n`;

  let md = HEADER;
  md += section(`## Source Summaries (\`${layout.wikiDir}/sources/\`)`, groups.source_summary, "_No source summaries ingested yet._");
  md += section(`## Entities (\`${layout.wikiDir}/entities/\`)`, groups.entity, "_No entity pages yet._");
  md += section(`## Concepts (\`${layout.wikiDir}/concepts/\`)`, groups.concept, "_No concept pages yet._");
  md += section(`## Syntheses & Analyses (\`${layout.wikiDir}/syntheses/\`)`, groups.synthesis, "_No synthesis pages yet._");
  if (groups.other.length) md += section("## Other Wiki Pages", groups.other, "");
  return md;
}
