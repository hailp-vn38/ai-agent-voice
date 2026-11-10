import { ItemView, Notice, type WorkspaceLeaf } from "obsidian";
import { createApp, h, ref, type App as VueApp } from "vue";
import { KNOWLEDGE_DARK_MODE_KEY, KNOWLEDGE_TRANSLATOR_KEY } from "@vela/knowledge-ui/integration";
import { knowledgeMessages } from "@vela/knowledge-ui/i18n";
import { collectionOf } from "../core/paths";
import type { WikiSession } from "../core/session";
import { tokenize } from "../core/wiki";
import { localTimestamp } from "../core/logMd";
import type WikiAgentPlugin from "../main";
import { confirmChange } from "./confirmModal";
import ChatPanel from "./ChatPanel.vue";
import { ObsidianChatState } from "./chatState";

export const VIEW_TYPE = "wiki-agent-chat";

export class ChatView extends ItemView {
  private session!: WikiSession;
  private chat!: ObsidianChatState;
  private approveAll = false;
  private vueApp: VueApp | null = null;
  private themeObserver: MutationObserver | null = null;

  constructor(leaf: WorkspaceLeaf, private readonly plugin: WikiAgentPlugin) { super(leaf); }
  getViewType(): string { return VIEW_TYPE; }
  getDisplayText(): string { return "Wiki Agent"; }
  getIcon(): string { return "book-open-check"; }

  async onOpen(): Promise<void> {
    this.resetSession();
    const root = this.contentEl;
    root.empty();
    root.addClass("wiki-agent-view", "knowledge-a");
    const mount = root.createDiv({ cls: "wiki-agent-chat-root" });
    this.vueApp = createApp({
      render: () => h(ChatPanel, {
        chat: this.chat,
        saveAnswer: () => { void this.saveAnswer().catch(error => this.showError(error)); },
        saveConversation: () => { void this.saveConversation().catch(error => this.showError(error)); },
        onOpenFile: (path: string) => { void this.app.workspace.openLinkText(this.plugin.vault.toVault(path), "", false); },
      }),
    });
    this.vueApp.provide(KNOWLEDGE_TRANSLATOR_KEY, (key, params) => {
      let text: string = knowledgeMessages.vi[key as keyof typeof knowledgeMessages.vi] ?? key;
      for (const [name, value] of Object.entries(params ?? {})) text = text.replaceAll(`{${name}}`, String(value));
      return text;
    });
    const dark = ref(root.ownerDocument.body.classList.contains("theme-dark"));
    this.vueApp.provide(KNOWLEDGE_DARK_MODE_KEY, dark);
    this.themeObserver = new MutationObserver(() => { dark.value = root.ownerDocument.body.classList.contains("theme-dark"); });
    this.themeObserver.observe(root.ownerDocument.body, { attributes: true, attributeFilter: ["class"] });
    this.vueApp.mount(mount);
    this.registerEvent(this.app.workspace.on("file-open", () => this.updateOpenFile()));
    this.registerEvent(this.app.workspace.on("active-leaf-change", () => this.updateOpenFile()));
    this.updateOpenFile();
    void this.plugin.store.ensureLoaded().then(() => this.updateOpenFile()).catch(error => this.showError(error));
  }

  async onClose(): Promise<void> {
    this.chat?.stop();
    this.themeObserver?.disconnect();
    this.themeObserver = null;
    this.vueApp?.unmount();
    this.vueApp = null;
  }

  resetSession(): void {
    this.approveAll = false;
    this.session = this.plugin.createSession(async change => {
      if (this.plugin.settings.autoApproveWrites || this.approveAll) return true;
      const decision = await confirmChange(this.app, change);
      if (decision === "approve_all") this.approveAll = true;
      return decision !== "reject";
    });
    if (this.chat) this.chat.reset(this.session);
    else this.chat = new ObsidianChatState(this.session, target => this.plugin.store.resolveLink(target), () => { this.approveAll = false; });
    this.updateOpenFile();
  }

  private updateOpenFile(): void {
    const file = this.app.workspace.getActiveFile();
    const path = file ? this.plugin.vault.fromVault(file.path) : null;
    this.chat.state.openFile = path && collectionOf(this.plugin.store.layout, path)
      ? { path, title: this.plugin.store.get(path)?.title ?? file!.basename } : null;
  }

  async runIngest(sourcePath: string): Promise<void> {
    if (this.chat.state.busy) { new Notice("Agent đang chạy. Hãy đợi hoặc bấm Dừng trước."); return; }
    await this.chat.ingest(sourcePath);
  }

  async postReport(title: string, markdown: string): Promise<void> {
    if (this.chat.state.busy) { new Notice("Agent đang chạy. Hãy đợi hoặc bấm Dừng trước."); return; }
    this.chat.report(title, markdown);
  }

  private showError(error: unknown): void { new Notice(error instanceof Error ? error.message : String(error)); }

  private slug(text: string): string {
    return tokenize(text).slice(0, 8).join("-").slice(0, 60) || "note";
  }

  private async saveAnswer(): Promise<void> {
    if (!this.chat.lastTurn?.answer) {
      new Notice("Chưa có câu trả lời nào để lưu.");
      return;
    }
    const { question, answer } = this.chat.lastTurn;
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
