import { requestUrl } from "obsidian";
import { AbortedError } from "../core/agent";
import type { ChatMessage, LlmClient, LlmReply, ToolCall, ToolSpec } from "../core/types";

export interface LlmConfig {
  baseUrl: string;
  model: string;
  apiKey: string;
}

interface WireToolCall {
  id?: string;
  function?: { name?: string; arguments?: string | Record<string, unknown> };
}

function toWire(message: ChatMessage) {
  if (message.role === "assistant" && message.tool_calls?.length) {
    return {
      role: "assistant",
      content: message.content,
      tool_calls: message.tool_calls.map((call) => ({
        id: call.id,
        type: "function",
        function: { name: call.name, arguments: call.arguments },
      })),
    };
  }
  if (message.role === "tool") return { role: "tool", tool_call_id: message.tool_call_id, content: message.content ?? "" };
  return { role: message.role, content: message.content ?? "" };
}

/**
 * OpenAI-compatible chat completions through Obsidian's `requestUrl`, which is
 * not subject to CORS and works on mobile. Responses are not streamed.
 */
export class OpenAiCompatClient implements LlmClient {
  constructor(private readonly config: () => LlmConfig) {}

  async chat(request: { messages: ChatMessage[]; tools: ToolSpec[]; signal?: AbortSignal }): Promise<LlmReply> {
    const { baseUrl, model, apiKey } = this.config();
    if (!baseUrl.trim() || !model.trim()) throw new Error("Chưa cấu hình Base URL hoặc Model trong phần cài đặt Wiki Agent.");
    const body: Record<string, unknown> = { model, messages: request.messages.map(toWire) };
    if (request.tools.length) {
      body.tools = request.tools.map((tool) => ({ type: "function", function: tool }));
      body.tool_choice = "auto";
    }
    const headers: Record<string, string> = { "Content-Type": "application/json" };
    if (apiKey) headers.Authorization = `Bearer ${apiKey}`;

    const pending = requestUrl({
      url: `${baseUrl.trim().replace(/\/+$/, "")}/chat/completions`,
      method: "POST",
      headers,
      body: JSON.stringify(body),
      throw: false,
    });
    // requestUrl cannot be cancelled; on abort we stop waiting and drop the reply.
    const response = await (request.signal
      ? Promise.race([
          pending,
          new Promise<never>((_, reject) => {
            const signal = request.signal as AbortSignal;
            if (signal.aborted) reject(new AbortedError());
            signal.addEventListener("abort", () => reject(new AbortedError()), { once: true });
          }),
        ])
      : pending);

    if (response.status < 200 || response.status >= 300) {
      let detail = "";
      try {
        const parsed = JSON.parse(response.text) as { error?: { message?: string } | string };
        detail = typeof parsed.error === "string" ? parsed.error : (parsed.error?.message ?? "");
      } catch {
        detail = response.text.slice(0, 300);
      }
      throw new Error(`LLM trả lỗi HTTP ${response.status}${detail ? `: ${detail}` : ""}`);
    }
    let data: { choices?: { message?: { content?: unknown; tool_calls?: WireToolCall[] } }[] };
    try {
      data = JSON.parse(response.text);
    } catch {
      throw new Error("Phản hồi của LLM không phải JSON hợp lệ.");
    }
    const message = data.choices?.[0]?.message;
    if (!message) throw new Error("Phản hồi của LLM không có nội dung.");
    const toolCalls: ToolCall[] = [];
    (message.tool_calls ?? []).forEach((call, index) => {
      const name = call.function?.name;
      if (!name) return;
      const args = call.function?.arguments;
      toolCalls.push({
        id: call.id || `call_${index}`,
        name,
        arguments: typeof args === "string" ? args : JSON.stringify(args ?? {}),
      });
    });
    return { content: typeof message.content === "string" ? message.content : null, toolCalls };
  }
}
