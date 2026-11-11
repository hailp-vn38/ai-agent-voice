export interface ParsedDoc {
  data: Record<string, unknown>;
  body: string;
  hasFrontmatter: boolean;
}

/**
 * Split leading YAML frontmatter from the body. Only the YAML subset wiki pages
 * use is understood: top-level scalars, inline arrays and block lists. Nested
 * maps (such as `qmd:` written by llm-wiki-rs) are skipped.
 */
export function parseDoc(text: string): ParsedDoc {
  const match = /^---\r?\n([\s\S]*?)\r?\n---[ \t]*(?:\r?\n|$)/.exec(text);
  if (!match) return { data: {}, body: text, hasFrontmatter: false };
  return { data: parseYamlSubset(match[1] ?? ""), body: text.slice(match[0].length), hasFrontmatter: true };
}

function parseYamlSubset(yaml: string): Record<string, unknown> {
  const data: Record<string, unknown> = {};
  let listKey: string | null = null;
  for (const line of yaml.split(/\r?\n/)) {
    if (!line.trim() || line.trimStart().startsWith("#")) continue;
    const item = /^\s+-\s+(.*)$/.exec(line) ?? /^-\s+(.*)$/.exec(line);
    if (item && listKey) {
      (data[listKey] as unknown[]).push(scalar(item[1] ?? ""));
      continue;
    }
    const pair = /^([A-Za-z0-9_-]+):\s*(.*)$/.exec(line);
    if (!pair) continue; // indented nested map content
    const key = pair[1] as string;
    const value = (pair[2] ?? "").trim();
    if (value === "") {
      data[key] = [];
      listKey = key;
    } else {
      data[key] = value.startsWith("[") && value.endsWith("]") ? inlineArray(value) : scalar(value);
      listKey = null;
    }
  }
  for (const [key, value] of Object.entries(data)) {
    if (Array.isArray(value) && value.length === 0) delete data[key];
  }
  return data;
}

function inlineArray(value: string): unknown[] {
  const inner = value.slice(1, -1).trim();
  if (!inner) return [];
  // Split on commas that are outside quotes and outside [[wikilinks]].
  const parts: string[] = [];
  let current = "";
  let quote = "";
  let depth = 0;
  for (const ch of inner) {
    if (quote) {
      if (ch === quote) quote = "";
    } else if (ch === '"' || ch === "'") {
      quote = ch;
    } else if (ch === "[") {
      depth++;
    } else if (ch === "]") {
      depth--;
    } else if (ch === "," && depth === 0) {
      parts.push(current);
      current = "";
      continue;
    }
    current += ch;
  }
  parts.push(current);
  return parts.map((part) => scalar(part.trim()));
}

function scalar(raw: string): unknown {
  const value = raw.trim();
  if (value.length >= 2 && value.startsWith('"') && value.endsWith('"')) {
    return value.slice(1, -1).replace(/\\"/g, '"').replace(/\\\\/g, "\\");
  }
  if (value.length >= 2 && value.startsWith("'") && value.endsWith("'")) {
    return value.slice(1, -1).replace(/''/g, "'");
  }
  if (value === "true") return true;
  if (value === "false") return false;
  if (/^-?\d+(\.\d+)?$/.test(value)) return Number(value);
  return value;
}

export function yamlString(value: string): string {
  return `"${value.replace(/\\/g, "\\\\").replace(/"/g, '\\"').replace(/\r?\n/g, " ")}"`;
}

export function asString(value: unknown): string | null {
  return typeof value === "string" && value.trim() ? value.trim() : null;
}

export function asStringList(value: unknown): string[] {
  if (Array.isArray(value)) return value.filter((item): item is string => typeof item === "string");
  return typeof value === "string" && value.trim() ? [value.trim()] : [];
}

/** `[[target|alias]]` or `[[target#heading]]` → `target`. Null when not a wikilink. */
export function wikilinkTarget(value: string): string | null {
  const match = /\[\[([^\]|#]+)/.exec(value);
  return match ? (match[1] as string).trim() : null;
}
