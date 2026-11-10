import { App, PluginSettingTab, Setting } from "obsidian";
import type WikiAgentPlugin from "./main";

export interface WikiAgentSettings {
  baseUrl: string;
  model: string;
  /** Only used when Obsidian has no secret storage. */
  apiKey: string;
  rootDir: string;
  sourcesDir: string;
  wikiDir: string;
  chatsDir: string;
  language: string;
  maxSteps: number;
  ingestMaxSteps: number;
  autoApproveWrites: boolean;
}

export const DEFAULT_SETTINGS: WikiAgentSettings = {
  baseUrl: "https://api.openai.com/v1",
  model: "",
  apiKey: "",
  rootDir: "",
  sourcesDir: "sources",
  wikiDir: "wiki",
  chatsDir: "chats",
  language: "auto",
  maxSteps: 12,
  ingestMaxSteps: 30,
  autoApproveWrites: false,
};

function cleanDir(value: string): string {
  return value.trim().replace(/\\/g, "/").replace(/^\/+|\/+$/g, "");
}

export class WikiAgentSettingTab extends PluginSettingTab {
  constructor(app: App, private readonly plugin: WikiAgentPlugin) {
    super(app, plugin);
  }

  display(): void {
    const { containerEl } = this;
    const { settings } = this.plugin;
    containerEl.empty();

    const save = (rebuild = false) => this.plugin.saveSettings(rebuild);
    const textSetting = (
      name: string,
      desc: string,
      key: "baseUrl" | "model" | "rootDir" | "sourcesDir" | "wikiDir" | "chatsDir" | "language",
      placeholder: string,
      rebuild = false,
    ) =>
      new Setting(containerEl).setName(name).setDesc(desc).addText((text) =>
        text.setPlaceholder(placeholder).setValue(settings[key]).onChange(async (value) => {
          const isDir = key.endsWith("Dir");
          settings[key] = isDir ? cleanDir(value) : value.trim();
          if (isDir && key !== "rootDir" && !settings[key]) settings[key] = DEFAULT_SETTINGS[key];
          await save(rebuild);
        }),
      );

    new Setting(containerEl).setName("Mô hình").setHeading();
    textSetting("Base URL", "Endpoint tương thích OpenAI, plugin gọi {Base URL}/chat/completions.", "baseUrl", DEFAULT_SETTINGS.baseUrl);
    textSetting("Model", "Tên model có hỗ trợ tool calling.", "model", "tên model");
    new Setting(containerEl)
      .setName("API key")
      .setDesc(
        this.plugin.hasSecretStorage()
          ? "Lưu trong kho bí mật của Obsidian."
          : "Lưu trong data.json của plugin. Đừng commit file này nếu vault đồng bộ bằng Git.",
      )
      .addText((text) => {
        text.inputEl.type = "password";
        text.setPlaceholder("sk-...").setValue(this.plugin.getApiKey()).onChange((value) => this.plugin.setApiKey(value.trim()));
      });

    new Setting(containerEl).setName("Vault").setHeading();
    textSetting("Thư mục gốc của wiki", "Để trống nếu wiki nằm ở gốc vault.", "rootDir", "vd: Knowledge", true);
    textSetting("Thư mục nguồn thô", "Nằm trong thư mục gốc của wiki.", "sourcesDir", "sources", true);
    textSetting("Thư mục wiki", "Nằm trong thư mục gốc của wiki.", "wikiDir", "wiki", true);
    textSetting("Thư mục lưu hội thoại", "Nằm trong thư mục gốc của wiki.", "chatsDir", "chats");

    new Setting(containerEl).setName("Agent").setHeading();
    textSetting("Ngôn ngữ trả lời", "\"auto\" để theo ngôn ngữ câu hỏi, hoặc tên ngôn ngữ như Vietnamese.", "language", "auto");
    const stepSetting = (name: string, desc: string, key: "maxSteps" | "ingestMaxSteps") =>
      new Setting(containerEl).setName(name).setDesc(desc).addText((text) =>
        text.setValue(String(settings[key])).onChange(async (value) => {
          const parsed = Number.parseInt(value, 10);
          if (Number.isFinite(parsed) && parsed >= 1 && parsed <= 100) {
            settings[key] = parsed;
            await save();
          }
        }),
      );
    stepSetting("Số bước tool mỗi lượt chat", "Sau số bước này agent phải trả lời.", "maxSteps");
    stepSetting("Số bước tool khi ingest", "Ingest một nguồn thường chạm nhiều trang.", "ingestMaxSteps");
    new Setting(containerEl)
      .setName("Tự duyệt thao tác ghi")
      .setDesc("Tắt: mỗi lần agent ghi, sửa, xoá file sẽ hiện hộp thoại có diff để bạn duyệt.")
      .addToggle((toggle) =>
        toggle.setValue(settings.autoApproveWrites).onChange(async (value) => {
          settings.autoApproveWrites = value;
          await save();
        }),
      );
  }
}
