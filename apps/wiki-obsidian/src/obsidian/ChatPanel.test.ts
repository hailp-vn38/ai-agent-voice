import { mount, flushPromises } from "@vue/test-utils";
import { describe, expect, it, vi } from "vitest";
import { KNOWLEDGE_TRANSLATOR_KEY } from "@vela/knowledge-ui/integration";
import { knowledgeMessages } from "@vela/knowledge-ui/i18n";
import { WikiSession } from "../core/session";
import { WikiStore } from "../core/wiki";
import { DEFAULT_LAYOUT, type LlmClient, type VaultPort } from "../core/types";
import ChatPanel from "./ChatPanel.vue";
import { ObsidianChatState } from "./chatState";

function setup(llm: LlmClient) {
  const files = new Map([["wiki/topic.md", "---\ntitle: Chủ đề\n---\nNội dung wiki."]]);
  const vault: VaultPort = {
    list: async () => [...files.keys()], read: async path => files.get(path) ?? null,
    write: async (path, text) => { files.set(path, text); }, remove: async path => files.delete(path), mtime: () => 0,
  };
  const store = new WikiStore(vault, DEFAULT_LAYOUT);
  const session = new WikiSession(store, llm, async () => true, { language: "auto", maxSteps: 3, ingestMaxSteps: 3 });
  const chat = new ObsidianChatState(session, target => store.resolveLink(target));
  const wrapper = mount(ChatPanel, {
    props: { chat, saveAnswer: vi.fn(), saveConversation: vi.fn() },
    global: { provide: { [KNOWLEDGE_TRANSLATOR_KEY as symbol]: (key: string) => knowledgeMessages.vi[key as keyof typeof knowledgeMessages.vi] ?? key } },
  });
  return { wrapper, chat, session };
}

describe("shared Wiki chat inside Obsidian", () => {
  it("sends a pinned file through the local agent, displays its tools and opens a citation", async () => {
    const requests: Parameters<LlmClient["chat"]>[0][] = [];
    const { wrapper, chat, session } = setup({ chat: async request => {
      requests.push(request);
      return requests.length === 1
        ? { content: null, toolCalls: [{ id: "read", name: "wiki_get", arguments: '{"file":"wiki/topic.md"}' }] }
        : { content: "Câu trả lời [[topic|Chủ đề]] và [[missing.md]].", toolCalls: [] };
    } });
    try {
      expect(wrapper.find(".wiki-chat-welcome").exists()).toBe(true);
      expect(wrapper.findAll(".wiki-chat-header-action")).toHaveLength(1);
      chat.state.openFile = { path: "wiki/topic.md", title: "Chủ đề" };
      await wrapper.vm.$nextTick();
      await wrapper.get(".wiki-chat-file-add").trigger("click");
      await wrapper.get("textarea").setValue("Giải thích chủ đề");
      await wrapper.get(".wiki-chat-compose").trigger("submit");
      await flushPromises();
      expect(requests[0]!.messages[0]!.content).toContain("wiki/topic.md");
      expect(wrapper.get(".wiki-chat-trail").text()).toContain("wiki/topic.md");
      expect(wrapper.get(".wiki-chat-answer").text()).toContain("Câu trả lời");
      expect(wrapper.findAll(".wiki-chat-citation")).toHaveLength(1);
      await wrapper.get(".wiki-chat-source-link").trigger("click");
      expect(wrapper.emitted("openFile")).toEqual([["wiki/topic.md"]]);
      expect(session.transcript).toHaveLength(2);
      await wrapper.get(".wiki-chat-header-action").trigger("click");
      expect(session.transcript).toHaveLength(0);
      expect(wrapper.find(".wiki-chat-welcome").exists()).toBe(true);
      expect(chat.state.pinnedFiles).toEqual([]);
    } finally { wrapper.unmount(); }
  });

  it("prevents duplicate submissions and displays stop without an error or stale reply", async () => {
    let finish!: (value: Awaited<ReturnType<LlmClient["chat"]>>) => void;
    const provider = vi.fn(() => new Promise<Awaited<ReturnType<LlmClient["chat"]>>>(resolve => { finish = resolve; }));
    const { wrapper, chat } = setup({ chat: provider });
    try {
      const turn = chat.send("Đợi phản hồi");
      await flushPromises();
      await chat.send("Gửi trùng");
      expect(provider).toHaveBeenCalledTimes(1);
      expect(wrapper.get("textarea").attributes("disabled")).toBeDefined();
      await wrapper.get(".wiki-chat-stop").trigger("click");
      finish({ content: "Phản hồi muộn", toolCalls: [] });
      await turn;
      await flushPromises();
      expect(chat.state.messages.at(-1)?.status).toBe("stopped");
      expect(wrapper.text()).not.toContain("Phản hồi muộn");
      expect(wrapper.find(".wiki-chat-error").exists()).toBe(false);
      expect(chat.state.busy).toBe(false);
    } finally { wrapper.unmount(); }
  });

  it("shows provider errors and keeps Ask read-only while Agent can write", async () => {
    const toolNames: string[][] = [];
    const { wrapper, chat } = setup({ chat: async request => {
      toolNames.push(request.tools.map(tool => tool.name));
      throw new Error("Không kết nối được model");
    } });
    try {
      await chat.send("Hỏi");
      expect(toolNames[0]).not.toContain("wiki_write_file");
      expect(chat.state.messages.at(-1)?.status).toBe("failed");
      expect(wrapper.text()).toContain("Không kết nối được model");
      expect(wrapper.find(".wiki-chat-error button").exists()).toBe(false);
      await wrapper.get("select").setValue("agent");
      await chat.send("Sửa");
      expect(toolNames[1]).toContain("wiki_write_file");
    } finally { wrapper.unmount(); }
  });

  it("drops callbacks from a reset turn and limits pinned files", async () => {
    let finish!: (value: Awaited<ReturnType<LlmClient["chat"]>>) => void;
    const { wrapper, chat, session } = setup({ chat: () => new Promise(resolve => { finish = resolve; }) });
    try {
      for (let i = 0; i < 6; i++) chat.pin({ path: `wiki/${i}.md`, title: String(i) });
      expect(chat.state.pinnedFiles).toHaveLength(4);
      const turn = chat.send("Cũ");
      await flushPromises();
      chat.reset();
      finish({ content: "Cũ", toolCalls: [] });
      await turn;
      expect(chat.state.messages).toEqual([]);
      expect(session.transcript).toEqual([]);
      expect(chat.state.busy).toBe(false);
    } finally { wrapper.unmount(); }
  });
});
