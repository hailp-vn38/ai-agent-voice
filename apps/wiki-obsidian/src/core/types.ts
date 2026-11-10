/** Seam between wiki logic and whatever owns the files (Obsidian, or memory in tests). */
export interface VaultPort {
  /** Every Markdown file, as a path relative to the wiki root. */
  list(): Promise<string[]>;
  read(path: string): Promise<string | null>;
  /** Creates parent folders as needed. */
  write(path: string, content: string): Promise<void>;
  remove(path: string): Promise<boolean>;
  mtime(path: string): number | null;
}

export interface Layout {
  sourcesDir: string;
  wikiDir: string;
  schemaPath: string;
}

export const DEFAULT_LAYOUT: Layout = {
  sourcesDir: "sources",
  wikiDir: "wiki",
  schemaPath: "SCHEMA.md",
};

export type Collection = "wiki" | "sources";

export interface ToolSpec {
  name: string;
  description: string;
  parameters: Record<string, unknown>;
}

export interface ToolCall {
  id: string;
  name: string;
  /** Raw JSON string as the model produced it. */
  arguments: string;
}

export interface ChatMessage {
  role: "system" | "user" | "assistant" | "tool";
  content: string | null;
  tool_calls?: ToolCall[];
  tool_call_id?: string;
}

export interface LlmReply {
  content: string | null;
  toolCalls: ToolCall[];
}

/** Seam between the agent loop and the model provider. */
export interface LlmClient {
  chat(request: {
    messages: ChatMessage[];
    tools: ToolSpec[];
    signal?: AbortSignal;
  }): Promise<LlmReply>;
}

/** A file change the agent wants to make. `after: null` is a deletion. */
export interface PendingChange {
  path: string;
  before: string | null;
  after: string | null;
}

export type ApproveChange = (change: PendingChange) => Promise<boolean>;
