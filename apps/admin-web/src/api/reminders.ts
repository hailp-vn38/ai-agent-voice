import { jsonRequest, requestJson, withQuery } from './client'
export interface Reminder { id: string; device_id: string; message: string; due_at_ms: number; timezone: string; status: string; row_revision: number; schedule_revision: number; sync_status: string }
export interface ReminderInput { message: string; delay_seconds?: number; scheduled_at?: string; timezone?: string }
export interface ReminderPage { items: Reminder[]; page: number; page_size: number; has_more: boolean; total: number; schedule_summary: { schedule_revision: number; last_acked_revision: number; session_connected: boolean | null; reminder_capable: boolean | null } }
const path = (id: string, agent?: string) => agent ? `/api/admin/agents/${encodeURIComponent(agent)}/devices/${encodeURIComponent(id)}/reminders` : `/api/admin/devices/${encodeURIComponent(id)}/reminders`
const policyPath = (id: string) => `/api/admin/agents/${encodeURIComponent(id)}/reminder-policy`
export interface ReminderPolicy { reminders_enabled: boolean; revision: number }
export interface AgentReminderQuery { device_id?: string; status?: string; due_from_ms?: number; due_to_ms?: number; page?: number; device_page?: number }
export interface AgentReminderPage extends Omit<ReminderPage,'schedule_summary'> { kpi: Record<'pending'|'completed'|'cancelled'|'expired',number>; devices: {device_id:string;name:string|null;enabled:boolean}[]; device_page:number;device_has_more:boolean }
export const remindersApi = {
 listAgent(key:string,query:AgentReminderQuery,signal?:AbortSignal) {return requestJson<AgentReminderPage>(withQuery(`/api/admin/agents/${encodeURIComponent(key)}/reminders`, {...query,page_size:50}),{}, {signal})},
 list(id: string, status = 'pending', page = 1, signal?: AbortSignal, agent?: string) { return requestJson<ReminderPage>(withQuery(path(id,agent), { status: status || undefined, page, page_size: 50 }), {}, { signal }) },
 create(id: string, input: ReminderInput, key: string, agent?: string) { return requestJson<Reminder>(path(id,agent), jsonRequest('POST',input), { headers: { 'Idempotency-Key': key } }) },
 update(device: string, id: string, revision: number, input: Partial<ReminderInput>, key: string, agent?: string) { return requestJson<Reminder>(`${path(device,agent)}/${encodeURIComponent(id)}`, jsonRequest('PATCH', input), { revision, headers: { 'Idempotency-Key':key } }) },
 cancel(device: string, id: string, revision: number, key: string, agent?: string) { return requestJson<Reminder>(`${path(device,agent)}/${encodeURIComponent(id)}/cancel`, jsonRequest('POST',{}), { revision, headers: { 'Idempotency-Key':key } }) },
 policy(id: string) { return requestJson<ReminderPolicy>(policyPath(id)) },
 setPolicy(id: string, enabled: boolean, revision: number) { return requestJson<ReminderPolicy>(policyPath(id), jsonRequest('PATCH',{reminders_enabled: enabled}), { revision }) },
}
