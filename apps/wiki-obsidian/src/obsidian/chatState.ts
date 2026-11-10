import { reactive } from "vue";
import { MAX_PINNED_FILES, type WikiChatFile, type WikiChatMessage } from "@vela/knowledge-ui/chat";
import { AbortedError, type AgentEvent, type AgentOutcome } from "../core/agent";
import type { ChatMode, WikiSession } from "../core/session";

type RunTurn = (options: { signal: AbortSignal; onEvent: (event: AgentEvent) => void }) => Promise<AgentOutcome>;

/** Adapts the local agent session to the same presentation model as web Wiki chat. */
export class ObsidianChatState {
  readonly state = reactive({
    messages: [] as WikiChatMessage[],
    busy: false,
    pinnedFiles: [] as WikiChatFile[],
    openFile: null as WikiChatFile | null,
    mode: "ask" as ChatMode,
  });
  lastTurn: { question: string; answer: string } | null = null;
  private controller: AbortController | null = null;
  private generation = 0;

  constructor(
    private session: WikiSession,
    private readonly resolveLink: (target: string) => string | null,
    private readonly beforeTurn: () => void = () => {},
  ) {}

  pin(file: WikiChatFile): void {
    if (this.state.pinnedFiles.length >= MAX_PINNED_FILES || this.state.pinnedFiles.some(item => item.path === file.path)) return;
    this.state.pinnedFiles.push(file);
  }

  unpin(path: string): void {
    this.state.pinnedFiles = this.state.pinnedFiles.filter(file => file.path !== path);
  }

  stop(): void {
    this.controller?.abort();
  }

  reset(session = this.session): void {
    this.stop();
    this.generation++;
    this.controller = null;
    this.session = session;
    session.reset();
    this.state.messages = [];
    this.state.pinnedFiles = [];
    this.state.busy = false;
    this.lastTurn = null;
  }

  async send(text: string): Promise<void> {
    if (!text.trim() || this.state.busy) return;
    const files = this.state.pinnedFiles.map(file => ({ ...file }));
    const session = this.session;
    const mode = this.state.mode;
    await this.run(text, options => session.send(text, mode, { ...options, pinned: files.map(file => file.path) }), files);
  }

  async ingest(path: string): Promise<void> {
    const session = this.session;
    await this.run(`Ingest \`${path}\``, options => session.ingest(path, options));
  }

  report(title: string, markdown: string): void {
    if (this.state.busy) return;
    this.state.messages.push(this.message("user", title), { ...this.message("assistant", markdown), citations: this.citationsOf(markdown) });
    this.session.note("user", title);
    this.session.note("assistant", markdown);
  }

  private message(role: WikiChatMessage["role"], text: string): WikiChatMessage {
    return { id: crypto.randomUUID(), role, text, status: "done", tools: [], graph: null, citations: [], errorCode: null };
  }

  private citationsOf(markdown: string): string[] {
    return [...new Set([...markdown.matchAll(/\[\[([^\]|#]+)(?:[^\]]*)\]\]/g)]
      .map(match => this.resolveLink(match[1]!.replace(/^qmd:\/\//, "").trim())).filter((path): path is string => Boolean(path)))];
  }

  private async run(question: string, run: RunTurn, files: WikiChatFile[] = []): Promise<void> {
    if (this.state.busy) return;
    this.beforeTurn();
    const controller = new AbortController();
    this.controller = controller;
    const generation = this.generation;
    this.state.busy = true;
    this.state.messages.push({ ...this.message("user", question), files }, this.message("assistant", ""));
    const message = this.state.messages[this.state.messages.length - 1]!;
    message.status = "streaming";
    try {
      const outcome = await run({
        signal: controller.signal,
        onEvent: event => {
          if (generation !== this.generation || controller.signal.aborted) return;
          if (event.type === "tool_start") {
            let query: string | undefined;
            try {
              const args = JSON.parse(event.call.arguments) as Record<string, unknown>;
              query = [args.query, args.file, args.pattern, args.rel_path, args.source_rel_path].find(value => typeof value === "string");
            } catch { /* Malformed arguments are reported by the tool executor. */ }
            message.tools.push({ call_id: event.call.id, name: event.call.name, query, state: "running", pages: [] });
          } else {
            const tool = message.tools.find(item => item.call_id === event.call.id);
            if (tool) tool.state = event.result.ok ? "completed" : "failed";
          }
        },
      });
      if (generation !== this.generation) return;
      if (controller.signal.aborted) throw new AbortedError();
      message.text = outcome.answer || "_Mô hình không trả về nội dung._";
      if (outcome.hitStepLimit) message.text += "\n\n> Đã chạm giới hạn số bước tool của lượt này.";
      message.status = "done";
      // Only existing vault files become clickable sources; unknown links remain text.
      message.citations = this.citationsOf(outcome.answer);
      this.lastTurn = { question, answer: outcome.answer };
    } catch (error) {
      if (generation !== this.generation) return;
      message.status = controller.signal.aborted || error instanceof AbortedError ? "stopped" : "failed";
      if (message.status === "failed") message.text = `Lỗi: ${error instanceof Error ? error.message : String(error)}`;
    } finally {
      if (generation === this.generation) {
        this.controller = null;
        this.state.busy = false;
      }
    }
  }
}
