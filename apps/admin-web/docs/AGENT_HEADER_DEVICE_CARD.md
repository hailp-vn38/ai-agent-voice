# Agent Header & Device Card — Option A

## UX design

- Agent Detail page structure, existing tabs and pipeline remain untouched.
- Replace only `AgentHeader.vue` with a compact identity header: back navigation; custom voice-agent icon; name, optional description; action buttons; template/device counts.
- Actions keep the existing `back`, `addDevice`, `editAgent` and `deleteAgent` events, with no change to data loading or API calls.
- In the Devices tab, `AgentDeviceList.vue` keeps its header and count but renders responsive `AgentDeviceCard.vue` entries (one column on mobile, two on tablet, three on wide screens).
- Device cards display a custom Voice AI device icon, device name/description, copyable Device ID, effective Template and Default/Override badge, plus edit/delete controls.
- Both icons are reusable Vue SVG components in `src/components/icons/`. They use `currentColor`, with semantic CSS tokens for light/dark themes. No raster assets or external image dependencies.

## Important device status semantics

The current admin store projects `AdminDevice.enabled` into legacy `Device.status = 'online' | 'offline'` values. This is **administrative admission**, not live WebSocket presence. Cards therefore display only **Connection permitted** / **Connection disabled** and explicitly avoid claiming the device is online. No offline/online live indicator is added.

There is no Device Detail route today, so the card itself is not a fake navigation target. Edit and Delete continue to emit the existing events wired by `AgentDetailPage.vue`.

## Validation

Run in `apps/admin-web`:

```sh
npm ci
npm run typecheck
npx vitest run src/components/agents/AgentHeader.test.ts src/components/agents/AgentDeviceCard.test.ts src/components/agents/AgentDeviceList.test.ts
npm run build
```

Check responsive widths, dark/light themes, a long Agent name, a long Device ID, copy ID (HTTPS/localhost), admission enabled/disabled, default/override Template, edit/delete dialog behavior, and keyboard navigation.

The full admin-web test suite currently contains unrelated ProviderCreateDrawer and SpeakerEnrollmentWizard expectation failures on the base branch; the dedicated `agent-ui` CI validates only this scoped feature while still running TypeScript and production build.
