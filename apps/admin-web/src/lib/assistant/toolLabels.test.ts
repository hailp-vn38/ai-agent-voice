import { expect, it } from 'vitest'
import { toolStatusLabel } from './toolLabels'
it('only renders fixed safe labels for System MCP and arbitrary tool names', () => {
  expect(toolStatusLabel('system_get_resources')).toBe('systemMcp.progress.system')
  expect(toolStatusLabel('mcp_discover_tools')).toBe('systemMcp.progress.discover')
  expect(toolStatusLabel('mcp_request_tool_review')).toBe('systemMcp.progress.propose')
  expect(toolStatusLabel('private-token-tool')).toBe('systemMcp.progress.tool')
})
