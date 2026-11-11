import { runAgent, type AgentEvent, type AgentOutcome } from "./agent";
import { sourceInnerPath } from "./paths";
import { agentSystemPrompt, askSystemPrompt, ingestSystemPrompt, type PromptContext } from "./prompts";
import { ToolExecutor, toolSpecs } from "./tools";
import type { ApproveChange, ChatMessage, LlmClient } from "./types";
import type { WikiStore } from "./wiki";

export type ChatMode = "ask" | "agent";

export interface SessionOptions {
  language: string;
  maxSteps: number;
  ingestMaxSteps: number;
}

export interface TurnOptions {
  pinned?: string[];
  signal?: AbortSignal;
  onEvent?: (event: AgentEvent) => void;
}

const INDEX_BUDGET = 8_000;
const SOURCE_SAMPLE_BUDGET = 16_000;
const HISTORY_BUDGET = 24_000;

/**
 * One conversation. Between turns only the questions and final answers are
 * kept; tool traces are dropped so the context stays small.
 */
export class WikiSession {
  private history: ChatMessage[] = [];

  constructor(
    private readonly store: WikiStore,
    private readonly llm: LlmClient,
    private readonly approve: ApproveChange,
    private readonly options: SessionOptions,
  ) {}

  get transcript(): readonly ChatMessage[] {
    return this.history;
  }

  reset(): void {
    this.history = [];
  }

  /** Put a message in the conversation without calling the model. */
  note(role: "user" | "assistant", content: string): void {
    this.history.push({ role, content });
  }

  private async context(sample: string, pinned?: string[]): Promise<PromptContext> {
    await this.store.ensureLoaded();
    const index = this.store.get(this.store.indexPath)?.text ?? "";
    return {
      layout: this.store.layout,
      schema: await this.store.readSchema(),
      index: index.length > INDEX_BUDGET ? `${index.slice(0, INDEX_BUDGET)}\n[Index truncated. Read the rest with wiki_get.]` : index,
      language: this.options.language,
      sample,
      pinned,
    };
  }

  private recentHistory(): ChatMessage[] {
    let size = 0;
    const kept: ChatMessage[] = [];
    for (let i = this.history.length - 1; i >= 0; i--) {
      const message = this.history[i] as ChatMessage;
      size += message.content?.length ?? 0;
      if (size > HISTORY_BUDGET && kept.length > 0) break;
      kept.unshift(message);
    }
    return kept;
  }

  private async turn(system: string, user: string, readOnly: boolean, maxSteps: number, options: TurnOptions): Promise<AgentOutcome> {
    const executor = new ToolExecutor(this.store, this.approve, { readOnly });
    const outcome = await runAgent({
      llm: this.llm,
      tools: toolSpecs(readOnly),
      runTool: (name, args) => executor.run(name, args),
      messages: [{ role: "system", content: system }, ...this.recentHistory(), { role: "user", content: user }],
      maxSteps,
      signal: options.signal,
      onEvent: options.onEvent,
    });
    this.history.push({ role: "user", content: user }, { role: "assistant", content: outcome.answer });
    return outcome;
  }

  async send(text: string, mode: ChatMode, options: TurnOptions = {}): Promise<AgentOutcome> {
    const ctx = await this.context(text, options.pinned);
    const system = mode === "ask" ? askSystemPrompt(ctx) : agentSystemPrompt(ctx);
    return this.turn(system, text, mode === "ask", this.options.maxSteps, options);
  }

  /** Compile one raw source into wiki pages. `sourcePath` is root-relative. */
  async ingest(sourcePath: string, options: TurnOptions = {}): Promise<AgentOutcome> {
    await this.store.ensureLoaded();
    const inner = sourceInnerPath(this.store.layout, sourcePath);
    const doc = inner ? this.store.get(`${this.store.layout.sourcesDir}/${inner}`) : undefined;
    if (!inner || !doc) throw new Error(`Không tìm thấy nguồn \`${sourcePath}\` trong \`${this.store.layout.sourcesDir}/\`.`);
    const truncated = doc.text.length > SOURCE_SAMPLE_BUDGET;
    const user =
      `Ingest the raw source \`${doc.path}\` into the wiki.\n\n` +
      `<source path="${doc.path}"${truncated ? ' truncated="true"' : ""}>\n${doc.text.slice(0, SOURCE_SAMPLE_BUDGET)}\n</source>` +
      (truncated ? `\n\nThe source is longer than shown. Read the rest with wiki_get before writing.` : "");
    const ctx = await this.context(doc.text);
    return this.turn(ingestSystemPrompt(ctx, inner), user, false, this.options.ingestMaxSteps, options);
  }
}
