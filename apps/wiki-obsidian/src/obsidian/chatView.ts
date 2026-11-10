import { ItemView, MarkdownRenderer, Notice, setIcon, type WorkspaceLeaf } from "obsidian";
import { AbortedError, type AgentEvent, type AgentOutcome } from "../core/agent";
import { collectionOf } from "../core/paths";
import type { ChatMode, WikiSession } from "../core/session";
import { tokenize } from "../core/wiki";
import { localTimestamp } from "../core/logMd";
import type WikiAgentPlugin from "../main";
import { confirmChange } from "./confirmModal";

export const VIEW_TYPE = "wiki-agent-chat";

const ARGUMENT_KEYS = ["file", "query", "rel_path", "source_rel_path", "pattern", "title", "scope"];

function describeCall(event: Extract<AgentEvent, { type: "tool_start" }>): string {
  try {
    const args = JSON.parse(event.call.arguments || "{}") as Record<string, unknown>;
    const key = ARGUMENT_KEYS.find((candidate) => typeof args[candidate] === "string");
    return key ? `${event.call.name} · ${String(args[key]).slice(0, 80)}` : event.call.name;
  } catch {
    return event.call.name;
  }
}

export class ChatView extends ItemView {
  private session!: WikiSession;
  private mode: ChatMode = "ask";
  private pinActive = false;
  private approveAll = false;
  private controller: AbortController | null = null;
  private lastTurn: { question: string; answer: string } | null = null;

  private messagesEl!: HTMLElement;
  private inputEl!: HTMLTextAreaElement;
  private sendEl!: HTMLButtonElement;
  private pinEl!: HTMLElement;

  constructor(leaf: WorkspaceLeaf, private readonly plugin: WikiAgentPlugin) {
    super(leaf);
  }

  getViewType(): string {
    return VIEW_TYPE;
  }

  getDisplayText(): string {
    return "Wiki Agent";
  }

  getIcon(): string {
    return "book-open-check";
  }

  async onOpen(): Promise<void> {
    this.resetSession();
    const root = this.contentEl;
    root.empty();
    root.addClass("wiki-agent-view");

    const bar = root.createDiv({ cls: "wiki-agent-bar" });
    const modeEl = bar.createEl("select", { cls: "dropdown" });
    modeEl.createEl("option", { value: "ask", text: "Hỏi (chỉ đọc)" });
    modeEl.createEl("option", { value: "agent", text: "Agent (được sửa wiki)" });
    modeEl.addEventListener("change", () => (this.mode = modeEl.value as ChatMode));

    const iconButton = (icon: string, label: string, onClick: () => void) => {
      const el = bar.createEl("button", { cls: "clickable-icon", attr: { "aria-label": label } });
      setIcon(el, icon);
      el.addEventListener("click", onClick);
      return el;
    };
    this.pinEl = iconButton("pin", "Ghim file đang mở làm ngữ cảnh", () => {
      this.pinActive = !this.pinActive;
      this.pinEl.toggleClass("is-active", this.pinActive);
    });
    iconButton("save", "Lưu câu trả lời cuối vào wiki/syntheses", () => void this.saveAnswer());
    iconButton("file-down", "Lưu hội thoại thành ghi chú", () => void this.saveConversation());
    iconButton("plus", "Cuộc trò chuyện mới", () => this.newChat());

    this.messagesEl = root.createDiv({ cls: "wiki-agent-messages" });
    this.messagesEl.addEventListener("click", (event) => this.onLinkClick(event));
    this.renderEmpty();

    const form = root.createDiv({ cls: "wiki-agent-form" });
    this.inputEl = form.createEl("textarea", { attr: { rows: "3", placeholder: "Hỏi wiki, hoặc giao việc cho agent…" } });
    this.inputEl.addEventListener("keydown", (event) => {
      if (event.key === "Enter" && !event.shiftKey && !event.isComposing) {
        event.preventDefault();
        void this.submit();
      }
    });
    this.sendEl = form.createEl("button", { text: "Gửi", cls: "mod-cta" });
    this.sendEl.addEventListener("click", () => (this.controller ? this.controller.abort() : void this.submit()));
  }

  async onClose(): Promise<void> {
    this.controller?.abort();
  }

  /** Called when settings that shape the store or model change. */
  resetSession(): void {
    this.controller?.abort();
    this.session = this.plugin.createSession(async (change) => {
      if (this.plugin.settings.autoApproveWrites || this.approveAll) return true;
      const decision = await confirmChange(this.app, change);
      if (decision === "approve_all") this.approveAll = true;
      return decision !== "reject";
    });
    this.lastTurn = null;
  }

  private newChat(): void {
    if (this.controller) return;
    this.session.reset();
    this.lastTurn = null;
    this.messagesEl.empty();
    this.renderEmpty();
  }

  private renderEmpty(): void {
    this.messagesEl.createDiv({
      cls: "wiki-agent-empty",
      text: "Hỏi về nội dung wiki, hoặc chuyển sang chế độ Agent để nhờ viết, sửa, ingest và lint các trang.",
    });
  }

  private addMessage(role: "user" | "assistant" | "error"): HTMLElement {
    this.messagesEl.querySelector(".wiki-agent-empty")?.remove();
    const el = this.messagesEl.createDiv({ cls: `wiki-agent-message wiki-agent-${role}` });
    el.scrollIntoView({ block: "end" });
    return el;
  }

  private async renderMarkdown(el: HTMLElement, markdown: string): Promise<void> {
    el.empty();
    await MarkdownRenderer.render(this.app, markdown, el, "", this);
    this.messagesEl.scrollTop = this.messagesEl.scrollHeight;
  }

  private onLinkClick(event: MouseEvent): void {
    const link = (event.target as HTMLElement).closest("a.internal-link");
    if (!link) return;
    event.preventDefault();
    const href = link.getAttribute("data-href") ?? link.getAttribute("href") ?? "";
    const target = href.split("#")[0] ?? "";
    const resolved = this.plugin.store.resolveLink(target);
    const linktext = resolved ? this.plugin.vault.toVault(resolved) : href;
    void this.app.workspace.openLinkText(linktext, "", event.ctrlKey || event.metaKey);
  }

  private pinnedPaths(): string[] {
    if (!this.pinActive) return [];
    const file = this.app.workspace.getActiveFile();
    const relative = file ? this.plugin.vault.fromVault(file.path) : null;
    if (!relative || !collectionOf(this.plugin.store.layout, relative)) {
      new Notice("File đang mở nằm ngoài thư mục nguồn và wiki nên không ghim được.");
      return [];
    }
    return [relative];
  }

  private async submit(): Promise<void> {
    const text = this.inputEl.value.trim();
    if (!text || this.controller) return;
    this.inputEl.value = "";
    const pinned = this.pinnedPaths();
    await this.runTurn(text, (options) => this.session.send(text, this.mode, { ...options, pinned }));
  }

  /** Start an ingest run for a root-relative source path. */
  async runIngest(sourcePath: string): Promise<void> {
    if (this.controller) {
      new Notice("Agent đang chạy. Hãy đợi hoặc bấm Dừng trước.");
      return;
    }
    await this.runTurn(`Ingest \`${sourcePath}\``, (options) => this.session.ingest(sourcePath, options));
  }

  /** Show a locally generated report and keep it in the conversation. */
  async postReport(title: string, markdown: string): Promise<void> {
    this.addMessage("user").setText(title);
    await this.renderMarkdown(this.addMessage("assistant"), markdown);
    this.session.note("user", title);
    this.session.note("assistant", markdown);
  }

  private async runTurn(
    shownQuestion: string,
    run: (options: { signal: AbortSignal; onEvent: (event: AgentEvent) => void }) => Promise<AgentOutcome>,
  ): Promise<void> {
    this.addMessage("user").setText(shownQuestion);
    const bubble = this.addMessage("assistant");
    const steps = bubble.createEl("details", { cls: "wiki-agent-steps" });
    const summary = steps.createEl("summary", { text: "Đang suy nghĩ…" });
    const body = bubble.createDiv();
    let count = 0;
    let current: HTMLElement | null = null;

    this.controller = new AbortController();
    this.approveAll = false;
    this.sendEl.setText("Dừng");
    this.sendEl.removeClass("mod-cta");
    try {
      const outcome = await run({
        signal: this.controller.signal,
        onEvent: (event) => {
          if (event.type === "tool_start") {
            count++;
            summary.setText(`Đang chạy tool (${count})…`);
            current = steps.createDiv({ cls: "wiki-agent-step", text: describeCall(event) });
          } else {
            current?.addClass(event.result.ok ? "is-ok" : "is-failed");
            if (!event.result.ok) current?.setAttribute("title", event.result.content.slice(0, 300));
          }
          this.messagesEl.scrollTop = this.messagesEl.scrollHeight;
        },
      });
      if (count === 0) steps.remove();
      else summary.setText(`${count} bước tool`);
      const answer = outcome.answer || "_Mô hình không trả về nội dung._";
      await this.renderMarkdown(body, outcome.hitStepLimit ? `${answer}\n\n> Đã chạm giới hạn số bước tool của lượt này.` : answer);
      this.lastTurn = { question: shownQuestion, answer: outcome.answer };
    } catch (error) {
      if (count === 0) steps.remove();
      else summary.setText(`${count} bước tool`);
      bubble.removeClass("wiki-agent-assistant");
      bubble.addClass("wiki-agent-error");
      body.setText(error instanceof AbortedError ? "Đã dừng." : `Lỗi: ${error instanceof Error ? error.message : String(error)}`);
    } finally {
      this.controller = null;
      this.sendEl.setText("Gửi");
      this.sendEl.addClass("mod-cta");
    }
  }

  private slug(text: string): string {
    return tokenize(text).slice(0, 8).join("-").slice(0, 60) || "note";
  }

  private async saveAnswer(): Promise<void> {
    if (!this.lastTurn?.answer) {
      new Notice("Chưa có câu trả lời nào để lưu.");
      return;
    }
    const { question, answer } = this.lastTurn;
    const title = question.replace(/\s+/g, " ").slice(0, 80);
    const result = await this.plugin.fileSynthesis({
      rel_path: `${this.slug(question)}.md`,
      query: question,
      title,
      one_line_summary: title,
      markdown_body: answer,
    });
    new Notice(result.ok ? result.content : `Không lưu được: ${result.content}`);
  }

  private async saveConversation(): Promise<void> {
    const messages = this.session.transcript;
    if (messages.length === 0) {
      new Notice("Cuộc trò chuyện đang trống.");
      return;
    }
    const now = new Date();
    const first = messages.find((message) => message.role === "user")?.content ?? "chat";
    const stamp = localTimestamp(now).replace(":", "");
    const path = `${this.plugin.settings.chatsDir}/${stamp} ${this.slug(first)}.md`;
    const body = messages
      .map((message) => `## ${message.role === "user" ? "Bạn" : "Wiki Agent"}\n\n${message.content ?? ""}`)
      .join("\n\n");
    const title = first.replace(/\s+/g, " ").slice(0, 80).replace(/"/g, "'");
    await this.plugin.vault.write(path, `---\ntitle: "${title}"\ncategory: "chat"\ncreated_at: "${now.toISOString()}"\n---\n\n${body}\n`);
    new Notice(`Đã lưu ${path}`);
    await this.app.workspace.openLinkText(this.plugin.vault.toVault(path), "", true);
  }
}
