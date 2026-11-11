import { Notice, Plugin, TFile, type TAbstractFile } from "obsidian";
import type { ToolResult } from "./core/agent";
import { formatLintReport, lintWiki } from "./core/lint";
import { collectionOf } from "./core/paths";
import { DEFAULT_SCHEMA_MD } from "./core/prompts";
import type { SynthesisRequest } from "./core/render";
import { WikiSession } from "./core/session";
import { ToolExecutor } from "./core/tools";
import type { ApproveChange, Layout } from "./core/types";
import { WikiStore } from "./core/wiki";
import { ChatView, VIEW_TYPE } from "./obsidian/chatView";
import { OpenAiCompatClient } from "./obsidian/llmClient";
import { ObsidianVault } from "./obsidian/vaultPort";
import { DEFAULT_SETTINGS, WikiAgentSettingTab, type WikiAgentSettings } from "./settings";

const SECRET_ID = "wiki-agent-api-key";

interface SecretStorageLike {
  getSecret(id: string): string | null;
  setSecret(id: string, secret: string): void;
}

export default class WikiAgentPlugin extends Plugin {
  settings: WikiAgentSettings = { ...DEFAULT_SETTINGS };
  vault!: ObsidianVault;
  store!: WikiStore;
  private readonly llm = new OpenAiCompatClient(() => ({
    baseUrl: this.settings.baseUrl,
    model: this.settings.model,
    apiKey: this.getApiKey(),
  }));

  async onload(): Promise<void> {
    this.settings = { ...DEFAULT_SETTINGS, ...((await this.loadData()) as Partial<WikiAgentSettings> | null) };
    this.buildServices();

    this.registerView(VIEW_TYPE, (leaf) => new ChatView(leaf, this));
    this.addRibbonIcon("book-open-check", "Wiki Agent", () => void this.openChat());
    this.addSettingTab(new WikiAgentSettingTab(this.app, this));

    this.addCommand({ id: "open-chat", name: "Mở chat", callback: () => void this.openChat() });
    this.addCommand({
      id: "ingest-current-file",
      name: "Ingest file hiện tại",
      checkCallback: (checking) => {
        const file = this.app.workspace.getActiveFile();
        const relative = file ? this.vault.fromVault(file.path) : null;
        const isSource = relative !== null && collectionOf(this.store.layout, relative) === "sources";
        if (!checking && isSource) void this.openChat().then((view) => view?.runIngest(relative));
        return isSource;
      },
    });
    this.addCommand({ id: "lint", name: "Lint wiki", callback: () => void this.runLint() });
    this.addCommand({ id: "rebuild-index", name: "Tạo lại wiki/index.md", callback: () => void this.rebuildIndex() });
    this.addCommand({ id: "init-wiki", name: "Khởi tạo cấu trúc wiki", callback: () => void this.initWiki() });

    // Obsidian fires `create` for every file while it loads the vault; listen after that.
    this.app.workspace.onLayoutReady(() => {
      const changed = (file: TAbstractFile) => {
        const relative = file instanceof TFile ? this.vault.fromVault(file.path) : null;
        if (relative) void this.store.refresh(relative);
      };
      this.registerEvent(this.app.vault.on("create", changed));
      this.registerEvent(this.app.vault.on("modify", changed));
      this.registerEvent(
        this.app.vault.on("delete", (file) => {
          const relative = this.vault.fromVault(file.path);
          if (relative) this.store.forget(relative);
        }),
      );
      this.registerEvent(
        this.app.vault.on("rename", (file, oldPath) => {
          const previous = this.vault.fromVault(oldPath);
          if (previous) this.store.forget(previous);
          changed(file);
        }),
      );
    });
  }

  private layout(): Layout {
    return { sourcesDir: this.settings.sourcesDir, wikiDir: this.settings.wikiDir, schemaPath: "SCHEMA.md" };
  }

  private buildServices(): void {
    this.vault = new ObsidianVault(this.app, this.settings.rootDir);
    this.store = new WikiStore(this.vault, this.layout());
  }

  /** `rebuild` when the folder layout changed and the store must be re-read. */
  async saveSettings(rebuild: boolean): Promise<void> {
    await this.saveData(this.settings);
    if (!rebuild) return;
    this.buildServices();
    for (const leaf of this.app.workspace.getLeavesOfType(VIEW_TYPE)) {
      if (leaf.view instanceof ChatView) leaf.view.resetSession();
    }
  }

  private secrets(): SecretStorageLike | null {
    const storage = (this.app as unknown as { secretStorage?: SecretStorageLike }).secretStorage;
    return storage && typeof storage.getSecret === "function" ? storage : null;
  }

  hasSecretStorage(): boolean {
    return this.secrets() !== null;
  }

  getApiKey(): string {
    return this.secrets()?.getSecret(SECRET_ID) ?? this.settings.apiKey;
  }

  async setApiKey(value: string): Promise<void> {
    const secrets = this.secrets();
    if (secrets) {
      secrets.setSecret(SECRET_ID, value);
      this.settings.apiKey = "";
    } else {
      this.settings.apiKey = value;
    }
    await this.saveData(this.settings);
  }

  createSession(approve: ApproveChange): WikiSession {
    return new WikiSession(this.store, this.llm, approve, {
      language: this.settings.language,
      maxSteps: this.settings.maxSteps,
      ingestMaxSteps: this.settings.ingestMaxSteps,
    });
  }

  /** File an answer the user chose to keep. The user asked for it, so no approval dialog. */
  fileSynthesis(request: SynthesisRequest): Promise<ToolResult> {
    return new ToolExecutor(this.store, async () => true).run("wiki_file_synthesis", JSON.stringify(request));
  }

  async openChat(): Promise<ChatView | null> {
    const { workspace } = this.app;
    let leaf = workspace.getLeavesOfType(VIEW_TYPE)[0] ?? null;
    if (!leaf) {
      leaf = workspace.getRightLeaf(false);
      await leaf?.setViewState({ type: VIEW_TYPE, active: true });
    }
    if (!leaf) return null;
    await workspace.revealLeaf(leaf);
    return leaf.view instanceof ChatView ? leaf.view : null;
  }

  private async runLint(): Promise<void> {
    await this.store.ensureLoaded();
    const report = formatLintReport(lintWiki(this.store));
    await this.store.appendLog("lint", "Wiki lint", []);
    const view = await this.openChat();
    await view?.postReport("Lint wiki", report);
  }

  private async rebuildIndex(): Promise<void> {
    await this.store.ensureLoaded();
    await this.store.rebuildIndexMd();
    new Notice(`Đã cập nhật ${this.store.indexPath}`);
  }

  private async initWiki(): Promise<void> {
    await this.store.ensureLoaded();
    const { layout } = this.store;
    if ((await this.vault.read(layout.schemaPath)) === null) await this.vault.write(layout.schemaPath, DEFAULT_SCHEMA_MD);
    if (!this.store.get(this.store.logPath)) await this.store.appendLog("init", "Wiki initialized", []);
    await this.store.rebuildIndexMd();
    for (const folder of [layout.sourcesDir, `${layout.wikiDir}/sources`, `${layout.wikiDir}/entities`, `${layout.wikiDir}/concepts`, `${layout.wikiDir}/syntheses`]) {
      const full = this.vault.toVault(folder);
      if (!this.app.vault.getAbstractFileByPath(full)) await this.app.vault.createFolder(full);
    }
    new Notice("Đã khởi tạo cấu trúc wiki.");
  }
}
