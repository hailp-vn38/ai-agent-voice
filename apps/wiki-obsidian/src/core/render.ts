import { parseDoc, yamlString } from "./frontmatter";
import { fileName } from "./paths";

export interface IngestRequest {
  source_rel_path: string;
  target_wiki_path?: string;
  raw_source_content?: string;
  title: string;
  one_line_summary: string;
  wiki_markdown_body: string;
  tags?: string[];
  related_entities?: string[];
  related_concepts?: string[];
}

export interface SynthesisRequest {
  rel_path: string;
  query: string;
  title: string;
  one_line_summary: string;
  markdown_body: string;
  tags?: string[];
}

function tagList(first: string, extra: string[] | undefined): string[] {
  const tags = [first];
  for (const raw of extra ?? []) {
    const tag = String(raw).trim().replace(/^#+/, "");
    if (tag && !tags.includes(tag)) tags.push(tag);
  }
  return tags;
}

function yamlList(items: string[]): string {
  return items.map((item) => `  - ${yamlString(item)}`).join("\n");
}

function oneLine(value: string): string {
  return value.replace(/\s*\r?\n\s*/g, " ").trim();
}

/** Summary page for a raw source: frontmatter that links back to the source, then the body. */
export function renderSourceSummary(
  req: IngestRequest,
  sourcePath: string,
  previousSources: string[],
  now: Date,
): string {
  const sources = [...new Set([...previousSources.map((path) => path.trim()).filter(Boolean), sourcePath])];
  const link = (path: string) => `[[${path}|${fileName(path)}]]`;
  const body = parseDoc(req.wiki_markdown_body).body.trim();
  const summary = oneLine(req.one_line_summary);

  let out =
    `---\ntitle: ${yamlString(req.title)}\ncategory: "source_summary"\nsummary: ${yamlString(summary)}\n` +
    `source_file: ${yamlString(link(sourcePath))}\nsource_count: ${sources.length}\n` +
    `sources:\n${yamlList(sources.map(link))}\ntags:\n${yamlList(tagList("wiki/source", req.tags))}\n` +
    `updated_at: ${yamlString(now.toISOString())}\n---\n\n`;
  if (!body.startsWith("# ")) out += `# ${oneLine(req.title)}\n\n`;
  out += `> **Raw Sources:** ${sources.map(link).join(", ")}  \n> **Summary:** ${summary}\n\n${body}\n`;

  const missing = (slugs: string[] | undefined, folder: string) =>
    (slugs ?? [])
      .map((slug) => String(slug).trim().replace(/^\[+|\]+$/g, ""))
      .filter((slug) => slug && ![`[[${slug}`, `[[${folder}/${slug}`, `[[wiki/${folder}/${slug}`].some((needle) => body.includes(needle)))
      .map((slug) => `[[${folder}/${slug}|${slug}]]`);
  const entities = missing(req.related_entities, "entities");
  const concepts = missing(req.related_concepts, "concepts");
  if (entities.length || concepts.length) {
    out += "\n## Cross-References\n";
    if (entities.length) out += `- **Entities:** ${entities.join(", ")}\n`;
    if (concepts.length) out += `- **Concepts:** ${concepts.join(", ")}\n`;
  }
  return out;
}

/** A query answer filed back into `wiki/syntheses/`. */
export function renderSynthesis(req: SynthesisRequest, now: Date): string {
  const body = parseDoc(req.markdown_body).body.trim();
  let out =
    `---\ntitle: ${yamlString(req.title)}\ncategory: "synthesis"\nsummary: ${yamlString(oneLine(req.one_line_summary))}\n` +
    `query: ${yamlString(oneLine(req.query))}\ntags:\n${yamlList(tagList("wiki/synthesis", req.tags))}\n` +
    `updated_at: ${yamlString(now.toISOString())}\n---\n\n`;
  if (!body.startsWith("# ")) out += `# ${oneLine(req.title)}\n\n`;
  out += `> **Query:** ${oneLine(req.query)}\n\n${body}\n`;
  return out;
}
