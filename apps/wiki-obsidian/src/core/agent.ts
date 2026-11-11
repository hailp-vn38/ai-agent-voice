import type { ChatMessage, LlmClient, ToolCall, ToolSpec } from "./types";

export interface ToolResult {
  ok: boolean;
  content: string;
}

export type AgentEvent =
  | { type: "tool_start"; call: ToolCall }
  | { type: "tool_end"; call: ToolCall; result: ToolResult };

export interface AgentRun {
  llm: LlmClient;
  tools: ToolSpec[];
  runTool: (name: string, rawArguments: string) => Promise<ToolResult>;
  /** Conversation so far, ending with the new user message. Not mutated. */
  messages: ChatMessage[];
  /** Tool rounds allowed before the model must answer. */
  maxSteps: number;
  signal?: AbortSignal;
  onEvent?: (event: AgentEvent) => void;
}

export interface AgentOutcome {
  answer: string;
  /** Input messages plus everything this run added. */
  messages: ChatMessage[];
  toolCalls: number;
  hitStepLimit: boolean;
}

export class AbortedError extends Error {
  constructor() {
    super("Đã dừng.");
    this.name = "AbortedError";
  }
}

/**
 * Tool-calling loop. After `maxSteps` tool rounds the model is called once
 * more without tools so it must answer from what it has.
 */
export async function runAgent(run: AgentRun): Promise<AgentOutcome> {
  const messages = [...run.messages];
  let toolCalls = 0;
  for (let step = 0; ; step++) {
    if (run.signal?.aborted) throw new AbortedError();
    const finalRound = step >= run.maxSteps;
    const reply = await run.llm.chat({
      messages,
      tools: finalRound ? [] : run.tools,
      signal: run.signal,
    });
    if (run.signal?.aborted) throw new AbortedError();
    if (finalRound || reply.toolCalls.length === 0) {
      const answer = reply.content?.trim() ?? "";
      messages.push({ role: "assistant", content: answer });
      return { answer, messages, toolCalls, hitStepLimit: finalRound && step > 0 };
    }
    messages.push({ role: "assistant", content: reply.content, tool_calls: reply.toolCalls });
    // Calls run in order: writes must not race each other.
    for (const call of reply.toolCalls) {
      if (run.signal?.aborted) throw new AbortedError();
      run.onEvent?.({ type: "tool_start", call });
      let result: ToolResult;
      try {
        result = await run.runTool(call.name, call.arguments);
      } catch (error) {
        result = { ok: false, content: `Tool failed: ${error instanceof Error ? error.message : String(error)}` };
      }
      toolCalls++;
      run.onEvent?.({ type: "tool_end", call, result });
      messages.push({ role: "tool", tool_call_id: call.id, content: result.content });
    }
  }
}
