import type { AdminJob, AgentRunView, JobArtifact, JobEvent, JobStep } from '@/api/types/agent-jobs'
import { projectLlmRuns, type LlmRun } from './llm-activity'

export type ActivityKind = 'llm' | 'plugin' | 'fork' | 'join' | 'agent' | 'job' | 'plan' | 'result'
export type ActivityStatus = 'running' | 'completed' | 'failed' | 'interrupted' | 'cancelled' | 'skipped' | 'unknown'
export type ActivityFilter =
  | { kind: 'all' }
  | { kind: 'llm' }
  | { kind: 'plugin' }
  | { kind: 'root' }
  | { kind: 'role'; role: string }
  | { kind: 'agent'; id: string }

export interface UnifiedActivity {
  id: string
  sequence: number
  time: string
  kind: ActivityKind
  status: ActivityStatus
  agentRunId: string | null
  role: string | null
  parentAgentRunId?: string | null
  agentRun?: AgentRunView
  forkGroupId?: string | null
  memberCount?: number | null
  childRuns?: AgentRunView[]
  title: string
  note: string
  assistantMessage?: string | null
  artifacts?: JobArtifact[]
  jobResult?: Record<string, unknown> | null
  llm?: LlmRun
  step?: JobStep
  event?: JobEvent
}

const count = (value: unknown): number | null =>
  typeof value === 'number' && Number.isSafeInteger(value) && value >= 0 ? value : null

const text = (value: unknown): string | null =>
  typeof value === 'string' && value.length > 0 ? value : null

function stepActivityStatus(step: JobStep | undefined, event: JobEvent): ActivityStatus {
  if (step?.attempt_state === 'unknown_outcome') return 'unknown'
  // The read view owns Step state; event payload is a fallback for absent Steps.
  const status = step?.status ?? text(event.payload.status)
  if (status === 'succeeded') return 'completed'
  if (status === 'failed' || status === 'cancelled' || status === 'skipped') return status
  return event.event_type === 'step_finished' && !status ? 'unknown' : 'running'
}

export function projectUnifiedActivity(job: AdminJob, events: JobEvent[]): UnifiedActivity[] {
  const ordered = [...events].sort((a, b) => a.sequence - b.sequence)
  const runs = projectLlmRuns(ordered, job.status, job.agent_runs)
  const steps = new Map(job.steps.map(step => [step.step_key, step]))
  const roles = new Map((job.agent_runs ?? []).map(run => [run.agent_run_id, run.role]))
  const agentRuns = new Map((job.agent_runs ?? []).map(run => [run.agent_run_id, run]))
  const items: UnifiedActivity[] = runs.map(run => {
    const first = run.events[0]
    const agentRunId = text(first?.payload.agent_run_id)
    const agentRun = agentRunId ? agentRuns.get(agentRunId) : undefined
    const hasTools = (agentRun?.tool_calls ?? 0) > 0
    const initialAssistantMessage = text(
      agentRun?.messages?.find(
        m =>
          m.role === 'assistant' &&
          m.text.trim().length > 0 &&
          (!hasTools || (m.tool_call_ids && m.tool_call_ids.length > 0)),
      )?.text,
    )
    const fallbackGoal = text(agentRun?.goal_excerpt)
    return {
      id: 'llm:' + run.id, sequence: run.startSequence, time: run.startedAt,
      kind: 'llm', status: run.status, agentRunId,
      role: agentRunId ? roles.get(agentRunId) ?? null : null,
      title: 'LLM Run ' + String(run.number).padStart(2, '0'),
      note: run.errorCode
        ? run.errorCode + (run.validationPath ? ' · ' + run.validationPath : '')
        : initialAssistantMessage ?? fallbackGoal ?? run.phase,
      assistantMessage: initialAssistantMessage,
      llm: run,
      step: run.stepKey ? steps.get(run.stepKey) : undefined,
    }
  })
  const llmStepKeys = new Set(runs.map(run => run.stepKey).filter((key): key is string => key !== null))
  const artifactsByStep = new Map<string, JobArtifact[]>()
  const artifactsBySession = new Map<string, JobArtifact[]>()
  for (const artifact of job.artifacts ?? []) {
    if (artifact.step_key) {
      const list = artifactsByStep.get(artifact.step_key) ?? []
      list.push(artifact)
      artifactsByStep.set(artifact.step_key, list)
    }
    if (artifact.producer_session_id) {
      const list = artifactsBySession.get(artifact.producer_session_id) ?? []
      list.push(artifact)
      artifactsBySession.set(artifact.producer_session_id, list)
    }
  }
  const pluginRows = new Map<string, UnifiedActivity>()
  for (const event of ordered) {
    const payload = event.payload ?? {}
    const agentRunId = text(payload.agent_run_id) ?? text(payload.root_agent_run_id) ??
      (event.event_type === 'job_finished'
        ? (job.agent_runs ?? []).find(run => run.role === 'orchestrator' || run.role === 'root' || !run.parent_agent_run_id)?.agent_run_id ?? null
        : null)
    const role = agentRunId ? roles.get(agentRunId) ?? null : null
    if (event.event_type === 'step_started' || event.event_type === 'step_finished') {
      if (!event.step_key) continue
      const step = steps.get(event.step_key)
      // Render an old Worker Step only when no traced LLM invocation represents it.
      if (step && !step.plugin_id && llmStepKeys.has(event.step_key)) continue
      const id = 'step:' + event.step_key
      const previous = pluginRows.get(id)
      if (previous) {
        if (event.event_type === 'step_finished') {
          previous.status = stepActivityStatus(step, event)
          previous.note = text(payload.error_code) ?? previous.note
        }
        continue
      }
      const assistantMessage = text(step?.assistant_message) ?? text(payload.assistant_message)
      const rawArgs = (step?.arguments ?? payload.arguments) as Record<string, unknown> | undefined
      const isSubagentStep = step?.operation === 'subagent' || payload.tool_name === 'subagent'
      const subagentPrompt = isSubagentStep
        ? text(rawArgs?.prompt) ?? text(rawArgs?.task)
        : null
      const stepArtifacts = artifactsByStep.get(event.step_key)
      const item: UnifiedActivity = {
        id, sequence: event.sequence, time: event.created_at, kind: step && !step.plugin_id ? 'llm' : 'plugin',
        status: stepActivityStatus(step, event),
        agentRunId: agentRunId ?? step?.agent_run_id ?? null,
        role: role ?? (step?.agent_run_id ? roles.get(step.agent_run_id) ?? null : null),
        title: step?.plugin_id ? step.plugin_id + '.' + (step.operation ?? 'unknown') : step ? 'Worker Step · ' + event.step_key : event.step_key,
        note:
          text(payload.error_code) ??
          assistantMessage ??
          subagentPrompt ??
          (step && !step.plugin_id ? 'Legacy worker execution' : 'Plugin operation'),
        assistantMessage,
        artifacts: stepArtifacts && stepArtifacts.length > 0 ? stepArtifacts : undefined,
        step, event,
      }
      pluginRows.set(id, item)
      items.push(item)
      continue
    }
    const make = (kind: ActivityKind, id: string, title: string, note: string, status: ActivityStatus = 'completed') =>
      items.push({ id, sequence: event.sequence, time: event.created_at, kind, status, agentRunId, role, title, note, event })
    if (event.event_type === 'agent_decision_accepted' && payload.action === 'delegate') {
      make('fork', 'fork:' + (text(payload.fork_group_id) ?? text(payload.turn_id) ?? text(payload.run_id) ?? event.sequence), 'Delegate / Fork', '')
    } else if (event.event_type === 'join_completed') {
      make('join', 'join:' + (text(payload.fork_group_id) ?? event.sequence), 'Join completed', 'Children: ' + (payload.member_count ?? '—'))
    } else if (event.event_type === 'agent_run_finished') {
      const agentRun = agentRunId ? agentRuns.get(agentRunId) : undefined
      const finalAssistantMessage =
        text(payload.assistant_message) ?? text(agentRun?.result?.text)
      const sessionArtifacts = agentRunId ? artifactsBySession.get(agentRunId) : undefined
      const status: ActivityStatus =
        payload.status === 'succeeded' || payload.status === 'completed'
          ? 'completed'
          : payload.status === 'cancelled'
            ? 'cancelled'
            : 'failed'
      items.push({
        id: 'agent:' + (agentRunId ?? event.sequence),
        sequence: event.sequence,
        time: event.created_at,
        kind: 'agent',
        status,
        agentRunId,
        role,
        title: 'Agent finished',
        note: text(payload.error_code) ?? finalAssistantMessage ?? 'Run checkpoint',
        assistantMessage: finalAssistantMessage,
        artifacts: sessionArtifacts && sessionArtifacts.length > 0 ? sessionArtifacts : undefined,
        event,
      })
    } else if (event.event_type === 'job_finished') {
      const resultText =
        text(job.result?.text) ?? text(job.result?.summary) ?? text(job.result?.markdown)
      const allArtifacts = job.artifacts && job.artifacts.length > 0 ? job.artifacts : undefined
      const hasJobResult = Boolean(job.result && Object.keys(job.result).length > 0)
      const status: ActivityStatus =
        payload.status === 'succeeded' || payload.status === 'partial_success'
          ? 'completed'
          : payload.status === 'cancelled'
            ? 'cancelled'
            : 'failed'
      items.push({
        id: 'job:terminal',
        sequence: event.sequence,
        time: event.created_at,
        kind: 'job',
        status,
        agentRunId,
        role,
        title: 'Job finished',
        note: text(payload.error_code) ?? text(payload.status) ?? 'Terminal checkpoint',
        assistantMessage: resultText,
        jobResult: job.result ?? null,
        artifacts: allArtifacts,
        event,
      })
      if (hasJobResult || allArtifacts) {
        items.push({
          id: 'job:result',
          sequence: event.sequence + 0.5,
          time: event.created_at,
          kind: 'result',
          status,
          agentRunId,
          role,
          title: 'Result',
          note: resultText ?? 'Job output',
          assistantMessage: resultText,
          jobResult: job.result ?? null,
          artifacts: allArtifacts,
          event,
        })
      }
    } else if (event.event_type === 'plan_accepted' || event.event_type === 'plan_rejected') {
      make('plan', 'plan:' + event.sequence, event.event_type === 'plan_accepted' ? 'Plan accepted' : 'Plan rejected',
        text(payload.code) ?? 'Legacy DAG', event.event_type === 'plan_rejected' ? 'failed' : 'completed')
    }
  }
  const created = new Map(ordered.filter(event => event.event_type === 'agent_run_created')
    .map(event => [text(event.payload.agent_run_id), event.payload]))
  for (const item of items) {
    const agent = item.agentRunId ? agentRuns.get(item.agentRunId) : undefined
    const creation = item.agentRunId ? created.get(item.agentRunId) : undefined
    item.agentRun = agent
    item.parentAgentRunId = agent?.parent_agent_run_id ?? text(creation?.parent_agent_run_id)
    item.role ??= text(creation?.role)
    const payload = item.event?.payload
    item.forkGroupId = item.kind === 'fork'
      ? text(payload?.fork_group_id) ?? text(payload?.turn_id) ?? text(payload?.run_id)
      : item.kind === 'join' ? text(payload?.fork_group_id)
        : agent?.fork_group_id ?? text(creation?.fork_group_id)
    if (item.kind !== 'fork' && item.kind !== 'join') continue
    const groupId = item.forkGroupId
    const childIds = new Set<string>()
    if (groupId) {
      for (const run of job.agent_runs ?? []) if (run.fork_group_id === groupId) childIds.add(run.agent_run_id)
      for (const [id, fields] of created) if (id && fields.fork_group_id === groupId) childIds.add(id)
    }
    const join = item.kind === 'join' ? item.event : groupId ? ordered.find(event => event.event_type === 'join_completed' && event.payload.fork_group_id === groupId) : undefined
    const fork = item.kind === 'fork' ? item.event : groupId ? ordered.find(event => event.event_type === 'agent_decision_accepted' && event.payload.action === 'delegate' &&
      (text(event.payload.fork_group_id) ?? text(event.payload.turn_id) ?? text(event.payload.run_id)) === groupId) : undefined
    item.memberCount = count(join?.payload.member_count) ?? count(fork?.payload.task_count) ?? (childIds.size || null)
    item.childRuns = [...childIds].flatMap(id => agentRuns.get(id) ? [agentRuns.get(id)!] : [])
    item.note = (item.kind === 'fork' ? 'Sub-agents: ' : 'Children: ') + (item.memberCount ?? '—')
  }
  return items.sort((a, b) => a.sequence - b.sequence)
}

export function visibleActivities(items: UnifiedActivity[], filter: ActivityFilter): UnifiedActivity[] {
  if (filter.kind === 'all') return items
  if (filter.kind === 'llm') return items.filter(item => item.kind === 'llm')
  if (filter.kind === 'plugin') return items.filter(item => item.kind === 'plugin')
  if (filter.kind === 'root') {
    return items.filter(
      item =>
        !item.parentAgentRunId &&
        (item.role === 'orchestrator' ||
          item.role === 'root' ||
          Boolean(item.role?.endsWith('_root')) ||
          item.agentRun?.run_index === 0 ||
          !item.agentRunId),
    )
  }
  if (filter.kind === 'agent') return items.filter(item => item.agentRunId === filter.id)
  return items.filter(item => item.role === filter.role)
}

export interface TimelineGroup {
  id: string
  child: boolean
  activities: UnifiedActivity[]
}

export function groupTimelineActivities(activities: UnifiedActivity[]): TimelineGroup[] {
  const groups: TimelineGroup[] = []
  const children = new Map<string, TimelineGroup>()
  for (const activity of activities) {
    if (!activity.parentAgentRunId || !activity.agentRunId) {
      groups.push({ id: activity.id, child: false, activities: [activity] })
      continue
    }
    let group = children.get(activity.agentRunId)
    if (!group) {
      group = { id: activity.agentRunId, child: true, activities: [] }
      children.set(activity.agentRunId, group)
      groups.push(group)
    }
    group.activities.push(activity)
  }
  return groups
}

export interface DagNodeLayout {
  id: string
  activity: UnifiedActivity
  x: number
  y: number
  width: number
  height: number
  laneId: string | null
}

export interface DagLaneLayout {
  id: string
  role: string | null
  theme: 'research' | 'document'
  group: TimelineGroup
  collapsed: boolean
  x: number
  y: number
  width: number
  height: number
  nodes: DagNodeLayout[]
}

export interface DagEdgeLayout {
  id: string
  fromId: string
  toId: string
  d: string
  kind: 'default' | 'branch' | 'pending' | 'animated'
}

export interface DagGroupRender {
  group: TimelineGroup
  lane: DagLaneLayout | null
  nodes: DagNodeLayout[]
}

export interface DagCanvasLayout {
  width: number
  height: number
  rootY: number
  groups: DagGroupRender[]
  lanes: DagLaneLayout[]
  nodes: DagNodeLayout[]
  edges: DagEdgeLayout[]
}

const NODE_W = 164
const BASE_NODE_H = 122
const EXTRA_NODE_H = 138
const RESULT_NODE_W = 420
const RESULT_NODE_H = 320
const NODE_ROW_GAP = 32
const LANE_HEADER_OFFSET = 62
const LANE_BOTTOM_PAD = 22
const LANE_SIDE_PAD = 24
const COLLAPSED_LANE_W = 256
const COLLAPSED_LANE_H = 88
const LANE_GAP = 46
const TOP_PAD = 48

function activityNodeWidth(act: UnifiedActivity): number {
  return act.kind === 'result' ? RESULT_NODE_W : NODE_W
}

function activityNodeHeight(act: UnifiedActivity): number {
  if (act.kind === 'result') return RESULT_NODE_H
  if (
    (act.llm && act.step) ||
    (act.childRuns && act.childRuns.length > 0) ||
    (act.artifacts && act.artifacts.length > 0)
  ) {
    return EXTRA_NODE_H
  }
  return BASE_NODE_H
}

function buildEdge(u: DagNodeLayout, v: DagNodeLayout): DagEdgeLayout {
  let d: string
  if (u.laneId !== null && u.laneId === v.laneId) {
    const sx = u.x + Math.round(u.width / 2)
    const sy = u.y + u.height
    const tx = v.x + Math.round(v.width / 2)
    const ty = v.y
    if (Math.abs(sx - tx) <= 2) {
      d = `M${sx} ${sy} L${tx} ${ty}`
    } else {
      const dy = ty - sy
      const c1y = Math.round(sy + dy * 0.45)
      const c2y = Math.round(sy + dy * 0.55)
      d = `M${sx} ${sy} C${sx} ${c1y},${tx} ${c2y},${tx} ${ty}`
    }
  } else {
    const sx = u.x + u.width
    const sy = u.y + Math.round(u.height / 2)
    const tx = v.x
    const ty = v.y + Math.round(v.height / 2)
    if (Math.abs(sy - ty) <= 2) {
      d = `M${sx} ${sy} L${tx} ${ty}`
    } else {
      const dx = tx - sx
      const c1x = Math.round(sx + dx * 0.45)
      const c2x = Math.round(sx + dx * 0.55)
      d = `M${sx} ${sy} C${c1x} ${sy},${c2x} ${ty},${tx} ${ty}`
    }
  }
  const status = v.activity.status
  const kind: DagEdgeLayout['kind'] =
    status === 'running'
      ? 'animated'
      : status === 'cancelled' || status === 'skipped'
        ? 'pending'
        : u.laneId !== null || v.laneId !== null
          ? 'branch'
          : 'default'
  return { id: `${u.id}->${v.id}`, fromId: u.id, toId: v.id, d, kind }
}

export function layoutDagCanvas(
  activities: UnifiedActivity[],
  collapsedLaneIds: ReadonlySet<string> = new Set(),
): DagCanvasLayout {
  const timelineGroups = groupTimelineActivities(activities)
  type Stage = { type: 'root'; group: TimelineGroup } | { type: 'branches'; groups: TimelineGroup[] }
  const stages: Stage[] = []
  for (const group of timelineGroups) {
    if (!group.child) {
      stages.push({ type: 'root', group })
      continue
    }
    const last = stages.at(-1)
    if (last?.type === 'branches') {
      last.groups.push(group)
    } else {
      stages.push({ type: 'branches', groups: [group] })
    }
  }

  function computeStageLaneGeometries(branchGroups: TimelineGroup[]) {
    const geometries: {
      laneY: number
      nodeY: number
      laneH: number
      firstNodeH: number
      collapsed: boolean
    }[] = []
    let currentY = TOP_PAD
    for (const [idx, group] of branchGroups.entries()) {
      const collapsed = collapsedLaneIds.has(group.id)
      const nodeHeights = group.activities.map(activityNodeHeight)
      const firstNodeH = nodeHeights[0] ?? BASE_NODE_H
      const stackH =
        nodeHeights.length > 0
          ? nodeHeights.reduce((sum, h) => sum + h, 0) + (nodeHeights.length - 1) * NODE_ROW_GAP
          : BASE_NODE_H
      const laneH = collapsed ? COLLAPSED_LANE_H : LANE_HEADER_OFFSET + stackH + LANE_BOTTOM_PAD
      if (branchGroups.length === 2 && idx === 1 && !collapsed && !collapsedLaneIds.has(branchGroups[0]!.id)) {
        currentY = Math.max(350, currentY)
      }
      const laneY = currentY
      const nodeY = laneY + LANE_HEADER_OFFSET
      geometries.push({ laneY, nodeY, laneH, firstNodeH, collapsed })
      currentY = laneY + laneH + LANE_GAP
    }
    return geometries
  }

  const branchGeometriesByStage = new Map<number, ReturnType<typeof computeStageLaneGeometries>>()
  const maxRootNodeH = Math.max(
    BASE_NODE_H,
    ...stages.filter(stage => stage.type === 'root').map(stage => activityNodeHeight(stage.group.activities[0]!)),
  )
  let maxStageBottom = maxRootNodeH + TOP_PAD * 2

  for (const [stageIdx, stage] of stages.entries()) {
    if (stage.type !== 'branches') continue
    const geoms = computeStageLaneGeometries(stage.groups)
    branchGeometriesByStage.set(stageIdx, geoms)
    const lastGeom = geoms.at(-1)
    if (lastGeom) {
      maxStageBottom = Math.max(maxStageBottom, lastGeom.laneY + lastGeom.laneH + TOP_PAD)
    }
  }

  const height = maxStageBottom
  const stageMidpoints: number[] = []

  for (const geoms of branchGeometriesByStage.values()) {
    const firstGeom = geoms[0]
    const lastGeom = geoms.at(-1)
    if (!firstGeom || !lastGeom) continue
    const stageSpanH = lastGeom.laneY + lastGeom.laneH - firstGeom.laneY
    const centeredTopY = Math.max(TOP_PAD, Math.round((height - stageSpanH) / 2))
    const shiftY = centeredTopY - firstGeom.laneY
    if (shiftY !== 0) {
      for (const geom of geoms) {
        geom.laneY += shiftY
        geom.nodeY += shiftY
      }
    }
    const firstCenter = firstGeom.collapsed
      ? firstGeom.laneY + Math.round(firstGeom.laneH / 2)
      : firstGeom.nodeY + Math.round(firstGeom.firstNodeH / 2)
    const lastCenter = lastGeom.collapsed
      ? lastGeom.laneY + Math.round(lastGeom.laneH / 2)
      : lastGeom.nodeY + Math.round(lastGeom.firstNodeH / 2)
    stageMidpoints.push(Math.round((firstCenter + lastCenter) / 2 - BASE_NODE_H / 2))
  }

  const defaultRootY = Math.max(TOP_PAD, Math.round((height - BASE_NODE_H) / 2))
  const rootY = stageMidpoints.length > 0 ? Math.max(TOP_PAD, stageMidpoints[0]!) : defaultRootY
  const rootCenterY = rootY + Math.round(BASE_NODE_H / 2)

  const groups: DagGroupRender[] = []
  const lanes: DagLaneLayout[] = []
  const nodes: DagNodeLayout[] = []
  const edges: DagEdgeLayout[] = []
  const stageBoundaries: { entry: DagNodeLayout[]; exit: DagNodeLayout[] }[] = []

  let cursorX = 40
  let maxRight = 0
  for (const [stageIdx, stage] of stages.entries()) {
    const nextStage = stages[stageIdx + 1]
    if (stage.type === 'root') {
      const act = stage.group.activities[0]!
      const nodeW = activityNodeWidth(act)
      const nodeH = activityNodeHeight(act)
      const node: DagNodeLayout = {
        id: act.id,
        activity: act,
        x: cursorX,
        y: Math.max(TOP_PAD, Math.round(rootCenterY - nodeH / 2)),
        width: nodeW,
        height: nodeH,
        laneId: null,
      }
      nodes.push(node)
      groups.push({ group: stage.group, lane: null, nodes: [node] })
      stageBoundaries.push({ entry: [node], exit: [node] })
      maxRight = Math.max(maxRight, node.x + node.width)
      cursorX += nodeW + (nextStage?.type === 'branches' ? 84 : 61)
      continue
    }

    const branchStartX = cursorX
    const entry: DagNodeLayout[] = []
    const exit: DagNodeLayout[] = []
    const geoms = branchGeometriesByStage.get(stageIdx) ?? computeStageLaneGeometries(stage.groups)
    let maxLaneRight = branchStartX + NODE_W

    for (const [laneIdx, group] of stage.groups.entries()) {
      const { laneY, nodeY, laneH, collapsed } = geoms[laneIdx]!
      const role = group.activities[0]?.role ?? null
      const theme: 'research' | 'document' =
        role === 'document' || role === 'wiki_curator' || role === 'wiki_ingestor' || laneIdx % 2 === 1
          ? 'document'
          : 'research'
      const laneX = branchStartX - LANE_SIDE_PAD
      const laneW = COLLAPSED_LANE_W
      const nodeX = laneX + Math.round((laneW - NODE_W) / 2)
      let currentNodeY = nodeY
      const laneNodes: DagNodeLayout[] = group.activities.map(act => {
        const height = activityNodeHeight(act)
        const node: DagNodeLayout = {
          id: act.id,
          activity: act,
          x: nodeX,
          y: currentNodeY,
          width: NODE_W,
          height,
          laneId: group.id,
        }
        currentNodeY += height + NODE_ROW_GAP
        return node
      })
      nodes.push(...laneNodes)
      maxLaneRight = Math.max(maxLaneRight, laneX + laneW)

      if (collapsed && laneNodes.length > 0) {
        const anchor: DagNodeLayout = {
          id: `lane:${group.id}`,
          activity: laneNodes[laneNodes.length - 1]!.activity,
          x: laneX,
          y: laneY,
          width: laneW,
          height: laneH,
          laneId: group.id,
        }
        entry.push(anchor)
        exit.push(anchor)
      } else if (laneNodes.length > 0) {
        entry.push(laneNodes[0]!)
        exit.push(laneNodes[laneNodes.length - 1]!)
        for (let i = 0; i < laneNodes.length - 1; i++) {
          edges.push(buildEdge(laneNodes[i]!, laneNodes[i + 1]!))
        }
      }

      const lane: DagLaneLayout = {
        id: group.id,
        role,
        theme,
        group,
        collapsed,
        x: laneX,
        y: laneY,
        width: laneW,
        height: laneH,
        nodes: laneNodes,
      }
      lanes.push(lane)
      groups.push({ group, lane, nodes: laneNodes })
    }
    stageBoundaries.push({ entry, exit })
    maxRight = Math.max(maxRight, maxLaneRight)
    cursorX = maxLaneRight + 60
  }

  for (let i = 0; i < stageBoundaries.length - 1; i++) {
    const current = stageBoundaries[i]!
    const next = stageBoundaries[i + 1]!
    for (const u of current.exit) {
      for (const v of next.entry) {
        edges.push(buildEdge(u, v))
      }
    }
  }

  const width = Math.max(NODE_W + 80, maxRight + 40)
  return { width, height, rootY, groups, lanes, nodes, edges }
}
