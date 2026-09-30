# 13: Database integration qualification

**What to build:** Release owner có một public-boundary qualification proving the complete database-enabled Voice flow and the unchanged database-disabled legacy Voice flow, with no unresolved specification dependency.

**Blocked by:** 08: Session-local Template switching; 10: Shared Tool-round Executor; 12: Readiness, shutdown và operational degradation; 14: RMCP Streamable HTTP protocol-engine migration.

**Status:** ready-for-agent

- [ ] End-to-end Admin API flow provisions Agent/Template/Provider/MCP/Device using only authenticated bounded public contracts and shows desired-versus-loaded restart semantics.
- [ ] WebSocket flow proves Device admission, immutable Effective Session Profile, valid template switch next-turn behavior, External MCP tool continuation and normal writer/history boundary without leaking sensitive content.
- [ ] Failure matrix proves unknown Device 403, invalid profile/runtime 503, optional MCP fail-soft, database contention/degradation behavior, strict Admin transport/auth and controlled shutdown semantics.
- [ ] Database-disabled and admission-disabled compatibility flows retain current Voice defaults and do not activate database semantics simply because migrations/schema exist.
- [ ] IntegrationHarness spawns the real production binary through the nonce-validated bound-address-file contract, uses only controlled process restart, and provisions a fresh temporary SQLite create-only scenario; stale/malformed ScenarioState or any resource conflict fails before a verify side effect.
- [ ] Deterministic Mock MCP remains in-process harness state only: `Arc<MockMcpStats>` plus bounded monotonic `Initialize`/`ToolsList`/`ToolsCall` event log proves discovery order without a stats endpoint; External `tools/call` behavior remains covered by the Tool-round gate.
- [ ] Required deterministic tests pass at application/router/WebSocket seams; real-model, external-server and hardware checks are separately reported optional evidence rather than blockers.
