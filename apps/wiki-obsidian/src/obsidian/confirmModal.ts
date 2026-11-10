import { Modal, type App } from "obsidian";
import { compactDiff, diffLines } from "../core/diff";
import type { PendingChange } from "../core/types";

export type Decision = "approve" | "approve_all" | "reject";

/** Shows what the agent wants to change and waits for the user's decision. */
export function confirmChange(app: App, change: PendingChange): Promise<Decision> {
  return new Promise((resolve) => new ConfirmChangeModal(app, change, resolve).open());
}

class ConfirmChangeModal extends Modal {
  private decision: Decision = "reject";

  constructor(app: App, private readonly change: PendingChange, private readonly done: (decision: Decision) => void) {
    super(app);
  }

  onOpen(): void {
    const { before, after, path } = this.change;
    const verb = after === null ? "Xoá" : before === null ? "Tạo" : "Sửa";
    this.setTitle(`${verb} ${path}?`);
    this.modalEl.addClass("wiki-agent-confirm");

    const pre = this.contentEl.createEl("pre", { cls: "wiki-agent-diff" });
    const addLine = (kind: string, text: string) => pre.createDiv({ cls: `wiki-agent-diff-${kind}`, text: text || " " });
    const lines = before !== null && after !== null ? diffLines(before, after) : null;
    if (lines) {
      for (const line of compactDiff(lines)) {
        if (line === null) addLine("gap", "⋯");
        else addLine(line.kind, `${line.kind === "add" ? "+" : line.kind === "del" ? "-" : " "} ${line.text}`);
      }
    } else {
      const kind = after === null ? "del" : "add";
      for (const text of (after ?? before ?? "").split("\n")) addLine(kind, `${kind === "add" ? "+" : "-"} ${text}`);
    }

    const buttons = this.contentEl.createDiv({ cls: "modal-button-container" });
    const button = (label: string, decision: Decision, cls?: string) => {
      const el = buttons.createEl("button", { text: label, cls });
      el.addEventListener("click", () => {
        this.decision = decision;
        this.close();
      });
    };
    button("Duyệt", "approve", "mod-cta");
    button("Duyệt hết lượt này", "approve_all");
    button("Từ chối", "reject");
  }

  onClose(): void {
    this.contentEl.empty();
    this.done(this.decision);
  }
}
