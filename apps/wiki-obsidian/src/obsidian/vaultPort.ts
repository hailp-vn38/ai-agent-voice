import { normalizePath, TFile, TFolder, type App } from "obsidian";
import type { VaultPort } from "../core/types";

/** `VaultPort` on top of Obsidian's vault, scoped to an optional root folder. */
export class ObsidianVault implements VaultPort {
  constructor(private readonly app: App, private readonly root: string) {}

  /** Root-relative path → vault path. */
  toVault(path: string): string {
    return normalizePath(this.root ? `${this.root}/${path}` : path);
  }

  /** Vault path → root-relative path, or null when outside the root. */
  fromVault(path: string): string | null {
    if (!this.root) return path;
    return path.startsWith(`${this.root}/`) ? path.slice(this.root.length + 1) : null;
  }

  private file(path: string): TFile | null {
    const found = this.app.vault.getAbstractFileByPath(this.toVault(path));
    return found instanceof TFile ? found : null;
  }

  async list(): Promise<string[]> {
    const paths: string[] = [];
    for (const file of this.app.vault.getMarkdownFiles()) {
      const relative = this.fromVault(file.path);
      if (relative) paths.push(relative);
    }
    return paths;
  }

  async read(path: string): Promise<string | null> {
    const file = this.file(path);
    return file ? this.app.vault.read(file) : null;
  }

  async write(path: string, content: string): Promise<void> {
    const file = this.file(path);
    if (file) {
      await this.app.vault.modify(file, content);
      return;
    }
    const full = this.toVault(path);
    const parts = full.split("/").slice(0, -1);
    let current = "";
    for (const part of parts) {
      current = current ? `${current}/${part}` : part;
      const existing = this.app.vault.getAbstractFileByPath(current);
      if (existing instanceof TFolder) continue;
      if (existing) throw new Error(`"${current}" là file, không tạo được thư mục.`);
      await this.app.vault.createFolder(current);
    }
    await this.app.vault.create(full, content);
  }

  /** Moves the file to the trash the user configured, so it can be restored. */
  async remove(path: string): Promise<boolean> {
    const file = this.file(path);
    if (!file) return false;
    await this.app.fileManager.trashFile(file);
    return true;
  }

  mtime(path: string): number | null {
    return this.file(path)?.stat.mtime ?? null;
  }
}
