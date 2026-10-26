import type { MessageKey } from '@/i18n/messages'
export function toolStatusLabel(name: string): MessageKey {
  if (name === 'system_get_status' || name === 'system_get_resources' || name === 'system_get_logs') return 'systemMcp.progress.system'
  if (name === 'mcp_discover_tools') return 'systemMcp.progress.discover'
  if (['mcp_list_servers', 'mcp_get_server', 'mcp_list_tools', 'mcp_get_agent_bindings'].includes(name)) return 'systemMcp.progress.read'
  if (['mcp_request_server_change', 'mcp_request_agent_binding_change', 'mcp_request_tool_review'].includes(name)) return 'systemMcp.progress.propose'
  return 'systemMcp.progress.tool'
}
