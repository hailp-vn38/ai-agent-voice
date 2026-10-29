import type { AdminJob, AgentRunView, JobArtifact, JobEvent, JobStep } from '@/api/types/agent-jobs'
import type { LlmRun } from './llm-activity'
import type { ActivityKind, ActivityStatus, JobMediaEmbed, UnifiedActivity } from './unified-activity'

export type ExecutionNodeType = 'root' | 'agent' | 'tool' | 'llm' | 'artifact' | 'result' | 'media'
export type ExecutionNodeState = 'done' | 'running' | 'pending' | 'failed'
export type ExecutionEventKind = 'start' | 'call' | 'message' | 'output' | 'error' | 'processing'

export interface ExecutionEventItem {
  time: string
  kind: ExecutionEventKind
  title: string
  detail: string
  validationPath?: string | null
  rawEvent?: JobEvent
}

export interface ExecutionTreeNode {
  id: string
  parent: string | null
  type: ExecutionNodeType
  name: string
  state: ExecutionNodeState
  rawStatus: string
  duration: string
  desc: string
  role: string | null
  agentRunId: string | null
  agentRun?: AgentRunView
  activity?: UnifiedActivity
  activityId?: string
  activityKind?: ActivityKind
  args?: Record<string, unknown> | null
  result?: unknown
  jobResult?: Record<string, unknown> | null
  assistantMessage?: string | null
  artifacts: JobArtifact[]
  mediaEmbeds: JobMediaEmbed[]
  llm?: LlmRun
  step?: JobStep
  errorCode?: string | null
  sortOrder: number
  events: ExecutionEventItem[]
}

export function parseTimestampMs(input: string | number | null | undefined): number | null {
  if (input === null || input === undefined) return null
  if (typeof input === 'number') {
    if (!Number.isFinite(input) || input <= 0) return null
    return input < 1e11 ? input * 1000 : input
  }
  const trimmed = input.trim()
  if (!trimmed) return null
  if (/^\d+$/.test(trimmed)) {
    const num = Number(trimmed)
    if (!Number.isFinite(num) || num <= 0) return null
    return trimmed.length <= 10 ? num * 1000 : num
  }
  const parsed = Date.parse(trimmed)
  return Number.isFinite(parsed) ? parsed : null
}

export function formatClockTime(input: string | number | null | undefined): string {
  const ms = parseTimestampMs(input)
  if (ms === null) return '—'
  return new Date(ms).toLocaleTimeString('en-GB', { hour12: false })
}

export function formatDurationMs(ms: number | null | undefined): string {
  if (ms === null || ms === undefined || !Number.isFinite(ms) || ms < 0) return '—'
  if (ms < 60_000) {
    const sec = ms / 1000
    return sec < 10 ? `${sec.toFixed(1).replace(/\.0$/, '')}s` : `${Math.round(sec)}s`
  }
  const totalSec = Math.floor(ms / 1000)
  const mins = Math.floor(totalSec / 60)
  const secs = totalSec % 60
  return `${String(mins).padStart(2, '0')}:${String(secs).padStart(2, '0')}`
}

export function formatSpanDuration(
  startInput: string | number | null | undefined,
  endInput: string | number | null | undefined,
  isRunning = false,
): string {
  const startMs = parseTimestampMs(startInput)
  const endMs = parseTimestampMs(endInput)
  if (startMs !== null && endMs !== null && endMs >= startMs) {
    return formatDurationMs(endMs - startMs)
  }
  if (isRunning) return 'live'
  return '—'
}

export function mapActivityStatusToNodeState(status: ActivityStatus | string | null | undefined): ExecutionNodeState {
  switch (status) {
    case 'completed':
    case 'succeeded':
    case 'partial_success':
    case 'done':
      return 'done'
    case 'running':
    case 'planning':
    case 'cancelling':
      return 'running'
    case 'failed':
    case 'unknown':
    case 'interrupted':
      return 'failed'
    case 'queued':
    case 'pending':
    case 'cancelled':
    case 'skipped':
    default:
      return 'pending'
  }
}

function isRootRoleName(role: string | null | undefined): boolean {
  if (!role) return false
  return role === 'orchestrator' || role === 'root' || role.endsWith('_root')
}

export function formatRoleDisplayName(role: string | null | undefined, isRoot = false): string {
  if (!role) return isRoot ? 'Root Orchestrator' : 'Sub-agent'
  switch (role) {
    case 'orchestrator':
    case 'root':
      return 'Root Orchestrator'
    case 'research':
      return isRoot ? 'Research' : 'Sub-agent · Research'
    case 'wiki_curator':
      return isRoot ? 'Wiki Curator' : 'Sub-agent · Wiki Curator'
    case 'document':
      return isRoot ? 'Document' : 'Sub-agent · Document'
    case 'source_collector':
      return isRoot ? 'Source Collector' : 'Sub-agent · Source Collector'
    case 'wiki_ingestor':
      return isRoot ? 'Wiki Ingestor' : 'Sub-agent · Wiki Ingestor'
    case 'general':
      return isRoot ? 'General' : 'Sub-agent · General'
    default:
      return isRoot ? role : `Sub-agent · ${role}`
  }
}

function safeStr(value: unknown): string | null {
  return typeof value === 'string' && value.trim().length > 0 ? value : null
}

function buildLlmEvents(run: LlmRun): ExecutionEventItem[] {
  return run.events.map(ev => {
    const time = formatClockTime(ev.created_at)
    switch (ev.event_type) {
      case 'llm_run_started':
      case 'agent_run_started':
        return {
          time,
          kind: 'start',
          title: 'LLM Run started',
          detail: `Role: ${run.role} · Phase: ${run.phase} · Provider: ${run.provider}`,
          rawEvent: ev,
        }
      case 'llm_provider_acquired':
        return {
          time,
          kind: 'call',
          title: 'Provider acquired',
          detail: `${run.model ?? run.provider}${ev.payload.provider_wait_ms !== undefined ? ` (${ev.payload.provider_wait_ms} ms)` : ''}`,
          rawEvent: ev,
        }
      case 'llm_request_started':
        return {
          time,
          kind: 'processing',
          title: 'Provider call started',
          detail:
            ev.payload.capacity_wait_ms !== undefined
              ? `Capacity wait: ${ev.payload.capacity_wait_ms} ms`
              : 'Request dispatched to model runtime.',
          rawEvent: ev,
        }
      case 'llm_response_received': {
        const tokens =
          run.inputTokens !== null || run.outputTokens !== null
            ? ` · Tokens: ${run.inputTokens ?? '—'} in / ${run.outputTokens ?? '—'} out`
            : ''
        return {
          time,
          kind: 'output',
          title: 'Response received',
          detail: `${ev.payload.request_duration_ms !== undefined ? `${ev.payload.request_duration_ms} ms` : 'Model responded'}${tokens}`,
          rawEvent: ev,
        }
      }
      case 'agent_decision_accepted':
        return {
          time,
          kind: 'output',
          title: 'Decision accepted',
          detail: safeStr(ev.payload.action) ? `Action: ${ev.payload.action}` : 'Valid agent decision.',
          rawEvent: ev,
        }
      case 'agent_decision_rejected':
      case 'plan_rejected': {
        const code = safeStr(ev.payload.validation_code) ?? safeStr(ev.payload.code) ?? 'validation_failed'
        const vPath = safeStr(ev.payload.validation_path)
        return {
          time,
          kind: 'error',
          title: 'Validation failed',
          detail: code,
          validationPath: vPath,
          rawEvent: ev,
        }
      }
      case 'llm_run_finished':
      case 'agent_run_finished': {
        const status = safeStr(ev.payload.status) ?? run.status
        const err = safeStr(ev.payload.error_code) ?? safeStr(ev.payload.validation_code)
        const isErr = status === 'failed' || Boolean(err)
        return {
          time,
          kind: isErr ? 'error' : 'output',
          title: 'Run finished',
          detail: err ?? `Status: ${status}`,
          validationPath: safeStr(ev.payload.validation_path),
          rawEvent: ev,
        }
      }
      default:
        return {
          time,
          kind: 'message',
          title: ev.event_type,
          detail: JSON.stringify(ev.payload),
          rawEvent: ev,
        }
    }
  })
}

export function buildExecutionTree(
  activities: UnifiedActivity[],
  job?: AdminJob | null,
): ExecutionTreeNode[] {
  if (!activities.length && !job) return []

  const agentRunMap = new Map<string, AgentRunView>()
  for (const run of job?.agent_runs ?? []) {
    agentRunMap.set(run.agent_run_id, run)
  }
  for (const act of activities) {
    if (act.agentRun && !agentRunMap.has(act.agentRun.agent_run_id)) {
      agentRunMap.set(act.agentRun.agent_run_id, act.agentRun)
    }
    for (const child of act.childRuns ?? []) {
      if (!agentRunMap.has(child.agent_run_id)) {
        agentRunMap.set(child.agent_run_id, child)
      }
    }
    if (act.agentRunId && !agentRunMap.has(act.agentRunId)) {
      agentRunMap.set(act.agentRunId, {
        agent_run_id: act.agentRunId,
        parent_agent_run_id: act.parentAgentRunId ?? null,
        role: act.role ?? 'general',
        goal_excerpt: '',
        status:
          act.status === 'completed'
            ? 'succeeded'
            : act.status === 'failed'
              ? 'failed'
              : act.status === 'cancelled'
                ? 'cancelled'
                : 'running',
        run_index: agentRunMap.size,
        llm_rounds: 0,
        tool_calls: 0,
        input_tokens: null,
        output_tokens: null,
        started_at: null,
        finished_at: null,
      })
    }
  }

  const allRuns = [...agentRunMap.values()].sort((a, b) => (a.run_index ?? 0) - (b.run_index ?? 0))
  const rootRun =
    allRuns.find(r => !r.parent_agent_run_id && (isRootRoleName(r.role) || r.run_index === 0)) ??
    allRuns.find(r => !r.parent_agent_run_id) ??
    null
  const rootRunId = rootRun?.agent_run_id ?? null

  const subagentRuns = allRuns.filter(r => r.agent_run_id !== rootRunId)
  const subagentIds = new Set(subagentRuns.map(r => r.agent_run_id))

  const agentFinishActivities = new Map<string, UnifiedActivity>()
  let jobTerminalActivity: UnifiedActivity | undefined
  for (const act of activities) {
    if (act.kind === 'agent' && act.agentRunId) {
      agentFinishActivities.set(act.agentRunId, act)
    } else if (act.kind === 'job') {
      jobTerminalActivity = act
    }
  }

  const rootFinishAct = rootRunId ? agentFinishActivities.get(rootRunId) : undefined
  const anyRunning =
    job?.status === 'running' ||
    job?.status === 'planning' ||
    job?.status === 'cancelling' ||
    activities.some(a => a.status === 'running')
  const anyFailed =
    job?.status === 'failed' ||
    (!job && activities.some(a => a.status === 'failed' || a.status === 'unknown'))

  const rootRawStatus =
    job?.status ??
    rootRun?.status ??
    rootFinishAct?.status ??
    jobTerminalActivity?.status ??
    (anyFailed ? 'failed' : anyRunning ? 'running' : 'succeeded')
  const rootState = mapActivityStatusToNodeState(rootRawStatus)

  const rootEvents: ExecutionEventItem[] = []
  const rootGoal =
    safeStr(rootRun?.goal_excerpt) ??
    safeStr(job?.goal) ??
    safeStr(rootFinishAct?.assistantMessage) ??
    safeStr(jobTerminalActivity?.assistantMessage) ??
    safeStr(jobTerminalActivity?.note) ??
    'Coordinate job execution'

  rootEvents.push({
    time: formatClockTime(rootRun?.started_at ?? job?.started_at ?? job?.created_at ?? activities[0]?.time),
    kind: 'start',
    title: 'Job started',
    detail: rootGoal,
  })

  for (const msg of rootRun?.messages ?? []) {
    if (msg.role === 'assistant' && msg.text.trim()) {
      rootEvents.push({
        time: formatClockTime(rootRun?.started_at ?? job?.created_at),
        kind: 'message',
        title: 'Agent message',
        detail: msg.text,
      })
    }
  }

  if (rootFinishAct) {
    rootEvents.push({
      time: formatClockTime(rootFinishAct.time),
      kind: rootFinishAct.status === 'failed' ? 'error' : 'output',
      title: rootFinishAct.title,
      detail: rootFinishAct.assistantMessage ?? rootFinishAct.note,
      rawEvent: rootFinishAct.event,
    })
  } else if (rootState === 'running') {
    rootEvents.push({
      time: formatClockTime(activities.at(-1)?.time ?? job?.created_at),
      kind: 'processing',
      title: 'Agent running',
      detail: 'Orchestrating subagents and tool execution...',
    })
  }

  const rootNode: ExecutionTreeNode = {
    id: 'root',
    parent: null,
    type: 'root',
    name: rootRun
      ? formatRoleDisplayName(rootRun.role, true)
      : safeStr(job?.title) ?? 'Root Orchestrator',
    state: rootState,
    rawStatus: rootRawStatus,
    duration: formatSpanDuration(
      rootRun?.started_at ?? job?.started_at ?? job?.created_at ?? activities[0]?.time,
      rootRun?.finished_at ?? job?.finished_at ?? (rootState !== 'running' ? activities.at(-1)?.time : null),
      rootState === 'running',
    ),
    desc: rootGoal,
    role: rootRun?.role ?? 'orchestrator',
    agentRunId: rootRunId,
    agentRun: rootRun ?? undefined,
    activity: rootFinishAct ?? jobTerminalActivity,
    activityId: rootFinishAct?.id ?? (rootRunId ? `agent:${rootRunId}` : 'root'),
    activityKind: rootFinishAct?.kind ?? 'agent',
    result: rootRun?.result ?? job?.result ?? null,
    jobResult: job?.result ?? rootFinishAct?.jobResult ?? jobTerminalActivity?.jobResult ?? null,
    assistantMessage:
      rootFinishAct?.assistantMessage ??
      jobTerminalActivity?.assistantMessage ??
      safeStr(rootRun?.result?.text) ??
      safeStr(job?.result?.text) ??
      safeStr(job?.result?.summary),
    artifacts: rootFinishAct?.artifacts ?? [],
    mediaEmbeds: [],
    errorCode: job?.error_code ?? null,
    sortOrder: 0,
    events: rootEvents,
  }

  const nodeMap = new Map<string, ExecutionTreeNode>()
  nodeMap.set('root', rootNode)

  for (const [idx, sub] of subagentRuns.entries()) {
    const subNodeId = `agent:${sub.agent_run_id}`
    const finishAct = agentFinishActivities.get(sub.agent_run_id)
    const subActs = activities.filter(a => a.agentRunId === sub.agent_run_id)
    const earliestSeq = subActs.reduce(
      (min, a) => Math.min(min, a.sequence),
      (sub.run_index ?? idx + 1) * 100,
    )
    const rawStatus =
      sub.status ??
      finishAct?.status ??
      (subActs.some(a => a.status === 'running')
        ? 'running'
        : subActs.some(a => a.status === 'failed')
          ? 'failed'
          : 'succeeded')
    const state = mapActivityStatusToNodeState(rawStatus)
    const parentId =
      sub.parent_agent_run_id &&
      sub.parent_agent_run_id !== rootRunId &&
      subagentIds.has(sub.parent_agent_run_id)
        ? `agent:${sub.parent_agent_run_id}`
        : 'root'

    const subDesc =
      safeStr(sub.goal_excerpt) ??
      safeStr(finishAct?.assistantMessage) ??
      safeStr(finishAct?.note) ??
      safeStr(sub.result?.summary) ??
      safeStr(sub.result?.text) ??
      `Subagent ${sub.role}`

    const subEvents: ExecutionEventItem[] = [
      {
        time: formatClockTime(sub.started_at ?? subActs[0]?.time),
        kind: 'start',
        title: parentId === 'root' ? 'Subagent spawned' : 'Nested subagent spawned',
        detail: subDesc,
      },
    ]

    for (const msg of sub.messages ?? []) {
      if (msg.role === 'assistant' && msg.text.trim()) {
        subEvents.push({
          time: formatClockTime(sub.started_at ?? subActs[0]?.time),
          kind: 'message',
          title: 'Agent message',
          detail: msg.text,
        })
      }
    }

    if (finishAct) {
      subEvents.push({
        time: formatClockTime(finishAct.time),
        kind: finishAct.status === 'failed' ? 'error' : 'output',
        title: finishAct.title, // 'Agent finished'
        detail:
          finishAct.assistantMessage ??
          safeStr(sub.result?.summary) ??
          safeStr(sub.result?.text) ??
          finishAct.note,
        rawEvent: finishAct.event,
      })
    } else if (state === 'running') {
      subEvents.push({
        time: formatClockTime(subActs.at(-1)?.time),
        kind: 'processing',
        title: 'Agent working',
        detail: 'Executing assigned task...',
      })
    }

    nodeMap.set(subNodeId, {
      id: subNodeId,
      parent: parentId,
      type: 'agent',
      name: formatRoleDisplayName(sub.role, false),
      state,
      rawStatus,
      duration: formatSpanDuration(
        sub.started_at ?? subActs[0]?.time,
        sub.finished_at ?? finishAct?.time,
        state === 'running',
      ),
      desc: subDesc,
      role: sub.role,
      agentRunId: sub.agent_run_id,
      agentRun: sub,
      activity: finishAct ?? subActs[0],
      activityId: finishAct?.id ?? subNodeId,
      activityKind: 'agent',
      result: sub.result ?? null,
      assistantMessage:
        finishAct?.assistantMessage ??
        safeStr(sub.result?.text) ??
        safeStr(sub.result?.summary),
      artifacts: finishAct?.artifacts ?? [],
      mediaEmbeds: [],
      sortOrder: earliestSeq,
      events: subEvents,
    })
  }

  for (const act of activities) {
    if (act.kind === 'agent') {
      // Already merged into root or subagent node above unless it has no matching node
      if (!act.agentRunId || act.agentRunId === rootRunId || nodeMap.has(`agent:${act.agentRunId}`)) {
        continue
      }
    }

    const parentId =
      act.agentRunId && act.agentRunId !== rootRunId && nodeMap.has(`agent:${act.agentRunId}`)
        ? `agent:${act.agentRunId}`
        : 'root'

    if (act.kind === 'llm') {
      const llm = act.llm
      const ms = llm ? (llm.totalDurationMs ?? llm.requestDurationMs) : null
      const state = mapActivityStatusToNodeState(act.status)
      const duration = ms !== null ? formatDurationMs(ms) : state === 'running' ? 'live' : '—'
      const events: ExecutionEventItem[] = llm ? buildLlmEvents(llm) : [
        {
          time: formatClockTime(act.time),
          kind: state === 'failed' ? 'error' : state === 'running' ? 'processing' : 'output',
          title: act.title,
          detail: act.note,
          rawEvent: act.event,
        },
      ]
      if (act.step && (act.step.status === 'failed' || act.step.attempt_state === 'unknown_outcome')) {
        events.push({
          time: formatClockTime(act.time),
          kind: 'error',
          title: `Worker checkpoint · ${act.step.step_key}`,
          detail: `${act.step.status}${act.step.attempt_state ? ' · ' + act.step.attempt_state : ''}${act.step.error_code ? ' · ' + act.step.error_code : ''}`,
        })
      }
      nodeMap.set(act.id, {
        id: act.id,
        parent: parentId,
        type: 'llm',
        name: llm?.model ? `${act.title} · ${llm.model}` : act.title,
        state,
        rawStatus: act.status,
        duration,
        desc: act.note,
        role: act.role,
        agentRunId: act.agentRunId,
        agentRun: act.agentRun,
        activity: act,
        activityId: act.id,
        activityKind: act.kind,
        assistantMessage: act.assistantMessage,
        artifacts: act.artifacts ?? [],
        mediaEmbeds: act.mediaEmbeds ?? [],
        llm,
        step: act.step,
        errorCode: llm?.errorCode ?? act.step?.error_code ?? null,
        sortOrder: act.sequence,
        events,
      })
      continue
    }

    if (act.kind === 'plugin') {
      const step = act.step
      const state = mapActivityStatusToNodeState(act.status)
      const events: ExecutionEventItem[] = [
        {
          time: formatClockTime(act.time),
          kind: 'call',
          title: 'Tool invocation',
          detail: step?.instruction || act.title,
          rawEvent: act.event,
        },
      ]
      if (act.assistantMessage) {
        events.push({
          time: formatClockTime(act.time),
          kind: 'message',
          title: 'Agent message',
          detail: act.assistantMessage,
        })
      }
      events.push({
        time: formatClockTime(act.time),
        kind: state === 'failed' ? 'error' : state === 'running' ? 'processing' : 'output',
        title: state === 'failed' ? 'Tool failed' : state === 'running' ? 'Streaming result' : 'Tool response',
        detail: step?.error_code ?? act.note,
        rawEvent: act.event,
      })

      nodeMap.set(act.id, {
        id: act.id,
        parent: parentId,
        type: 'tool',
        name: act.title,
        state,
        rawStatus: act.status,
        duration: state === 'running' ? 'live' : formatClockTime(act.time),
        desc: step?.instruction || act.note,
        role: act.role,
        agentRunId: act.agentRunId,
        agentRun: act.agentRun,
        activity: act,
        activityId: act.id,
        activityKind: act.kind,
        args: step?.arguments ?? null,
        result: step?.result ?? null,
        assistantMessage: act.assistantMessage,
        artifacts: act.artifacts ?? [],
        mediaEmbeds: act.mediaEmbeds ?? [],
        step,
        errorCode: step?.error_code ?? null,
        sortOrder: act.sequence,
        events,
      })
      continue
    }

    if (act.kind === 'result') {
      const state = mapActivityStatusToNodeState(act.status)
      nodeMap.set(act.id, {
        id: act.id,
        parent: 'root',
        type: 'result',
        name: act.title,
        state,
        rawStatus: act.status,
        duration: formatClockTime(act.time),
        desc: act.note,
        role: act.role,
        agentRunId: act.agentRunId,
        agentRun: act.agentRun,
        activity: act,
        activityId: act.id,
        activityKind: act.kind,
        result: act.jobResult ?? null,
        jobResult: act.jobResult ?? null,
        assistantMessage: act.assistantMessage,
        artifacts: act.artifacts ?? [],
        mediaEmbeds: act.mediaEmbeds ?? [],
        sortOrder: act.sequence + 10_000,
        events: [
          {
            time: formatClockTime(act.time),
            kind: state === 'failed' ? 'error' : 'output',
            title: act.title,
            detail: act.note,
            rawEvent: act.event,
          },
        ],
      })
      continue
    }

    if (act.kind === 'media') {
      const state = mapActivityStatusToNodeState(act.status)
      nodeMap.set(act.id, {
        id: act.id,
        parent: 'root',
        type: 'media',
        name: act.title,
        state,
        rawStatus: act.status,
        duration: `${(act.artifacts?.length ?? 0) + (act.mediaEmbeds?.length ?? 0)} items`,
        desc: act.note,
        role: act.role,
        agentRunId: act.agentRunId,
        agentRun: act.agentRun,
        activity: act,
        activityId: act.id,
        activityKind: act.kind,
        artifacts: act.artifacts ?? [],
        mediaEmbeds: act.mediaEmbeds ?? [],
        sortOrder: act.sequence + 10_001,
        events: [
          {
            time: formatClockTime(act.time),
            kind: 'output',
            title: act.title,
            detail: act.note,
            rawEvent: act.event,
          },
        ],
      })
      continue
    }

    // fork, join, plan, job:terminal
    const state = mapActivityStatusToNodeState(act.status)
    const childSummary =
      act.childRuns && act.childRuns.length > 0
        ? act.childRuns
            .map(c => `${formatRoleDisplayName(c.role, false)} · ${c.goal_excerpt} (${c.agent_run_id})`)
            .join('\n')
        : null
    nodeMap.set(act.id, {
      id: act.id,
      parent: parentId,
      type: act.kind === 'job' ? 'artifact' : 'tool',
      name: act.title,
      state,
      rawStatus: act.status,
      duration:
        act.kind === 'fork' || act.kind === 'join'
          ? `${act.memberCount ?? 0} children`
          : formatClockTime(act.time),
      desc: childSummary ? `${act.note}\n${childSummary}` : act.note,
      role: act.role,
      agentRunId: act.agentRunId,
      agentRun: act.agentRun,
      activity: act,
      activityId: act.id,
      activityKind: act.kind,
      result: act.jobResult ?? null,
      jobResult: act.jobResult ?? null,
      assistantMessage: act.assistantMessage,
      artifacts: act.artifacts ?? [],
      mediaEmbeds: act.mediaEmbeds ?? [],
      sortOrder: act.kind === 'job' ? act.sequence + 9_000 : act.sequence,
      events: [
        {
          time: formatClockTime(act.time),
          kind: state === 'failed' ? 'error' : 'output',
          title: act.title,
          detail: childSummary ? `${act.note} — ${childSummary}` : act.note,
          rawEvent: act.event,
        },
      ],
    })
  }

  // Depth-first pre-order traversal so parent is immediately followed by its subtree
  const childrenByParent = new Map<string | null, ExecutionTreeNode[]>()
  for (const node of nodeMap.values()) {
    const list = childrenByParent.get(node.parent) ?? []
    list.push(node)
    childrenByParent.set(node.parent, list)
  }
  for (const list of childrenByParent.values()) {
    list.sort((a, b) => a.sortOrder - b.sortOrder)
  }

  const ordered: ExecutionTreeNode[] = []
  const visited = new Set<string>()
  function visit(node: ExecutionTreeNode) {
    if (visited.has(node.id)) return
    visited.add(node.id)
    ordered.push(node)
    for (const child of childrenByParent.get(node.id) ?? []) {
      visit(child)
    }
  }
  for (const top of childrenByParent.get(null) ?? []) {
    visit(top)
  }
  for (const leftover of nodeMap.values()) {
    if (!visited.has(leftover.id)) visit(leftover)
  }

  return ordered
}

export function treeNodeAncestors(nodes: ExecutionTreeNode[], id: string): ExecutionTreeNode[] {
  const byId = new Map(nodes.map(n => [n.id, n]))
  const out: ExecutionTreeNode[] = []
  const seen = new Set<string>()
  let cur = byId.get(id)
  while (cur && cur.parent && !seen.has(cur.parent)) {
    seen.add(cur.parent)
    const parent = byId.get(cur.parent)
    if (parent) {
      out.unshift(parent)
      cur = parent
    } else {
      break
    }
  }
  return out
}

export function treeNodeChildren(nodes: ExecutionTreeNode[], id: string): ExecutionTreeNode[] {
  return nodes.filter(n => n.parent === id)
}

export function treeNodeDescendants(nodes: ExecutionTreeNode[], id: string): ExecutionTreeNode[] {
  const out: ExecutionTreeNode[] = []
  const seen = new Set<string>()
  function walk(parentId: string) {
    for (const child of nodes) {
      if (child.parent === parentId && !seen.has(child.id)) {
        seen.add(child.id)
        out.push(child)
        walk(child.id)
      }
    }
  }
  walk(id)
  return out
}

export function filterVisibleTreeNodes(
  nodes: ExecutionTreeNode[],
  expanded: ReadonlySet<string>,
  searchQuery: string,
): ExecutionTreeNode[] {
  const q = searchQuery.trim().toLowerCase()
  if (q) {
    const matchingIds = new Set<string>()
    for (const n of nodes) {
      const hay = `${n.name} ${n.type} ${n.role ?? ''} ${n.desc} ${n.step?.step_key ?? ''}`.toLowerCase()
      if (hay.includes(q)) {
        matchingIds.add(n.id)
        for (const anc of treeNodeAncestors(nodes, n.id)) {
          matchingIds.add(anc.id)
        }
      }
    }
    return nodes.filter(n => matchingIds.has(n.id))
  }
  return nodes.filter(n => treeNodeAncestors(nodes, n.id).every(a => expanded.has(a.id)))
}
