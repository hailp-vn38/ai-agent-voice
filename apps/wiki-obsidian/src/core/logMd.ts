export const LOG_MD_HEADER =
  '---\ntitle: "Wiki Activity Log"\ncategory: "system"\ntags:\n  - "wiki/system"\n  - "wiki/log"\n---\n\n# Wiki Evolution Log\n\n';

export interface LogEntry {
  timestamp: string;
  action: string;
  title: string;
  details: string[];
}

function pad(value: number): string {
  return String(value).padStart(2, "0");
}

export function localTimestamp(date: Date): string {
  return `${date.getFullYear()}-${pad(date.getMonth() + 1)}-${pad(date.getDate())} ${pad(date.getHours())}:${pad(date.getMinutes())}`;
}

/** Append `## [YYYY-MM-DD HH:MM] action | title` plus bullet details. */
export function appendLogEntry(
  current: string | null,
  action: string,
  title: string,
  details: string[],
  now: Date,
): string {
  let text = current ?? LOG_MD_HEADER;
  if (!text.endsWith("\n")) text += "\n";
  const oneLine = (value: string) => value.replace(/\s*\r?\n\s*/g, " ").trim();
  let block = `## [${localTimestamp(now)}] ${oneLine(action)} | ${oneLine(title)}\n`;
  for (const detail of details) block += `- ${oneLine(detail)}\n`;
  return `${text}${block}\n`;
}

/** Most recent entries first. */
export function parseLogEntries(log: string, limit: number): LogEntry[] {
  const entries: LogEntry[] = [];
  let current: LogEntry | null = null;
  for (const raw of log.split("\n")) {
    const line = raw.trim();
    const heading = /^## \[([^\]]*)\](.*)$/.exec(line);
    if (heading) {
      if (current) entries.push(current);
      const rest = heading[2] ?? "";
      const bar = rest.indexOf("|");
      current = {
        timestamp: (heading[1] ?? "").trim(),
        action: bar >= 0 ? rest.slice(0, bar).trim() : "log",
        title: bar >= 0 ? rest.slice(bar + 1).trim() : rest.trim(),
        details: [],
      };
    } else if (current && line) {
      current.details.push(line.startsWith("- ") ? line.slice(2).trim() : line);
    }
  }
  if (current) entries.push(current);
  entries.reverse();
  return limit > 0 ? entries.slice(0, limit) : entries;
}
