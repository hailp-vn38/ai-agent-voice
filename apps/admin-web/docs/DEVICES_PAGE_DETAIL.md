# Devices Page and Device Detail implementation

## Design and navigation

- \`/devices\` uses a responsive shared DeviceCard grid instead of the legacy table. Full card navigates to \`/devices/:deviceId\`; embedded Edit/Delete/Copy buttons remain independent and keyboard accessible.
- Summary shows registered devices and administratively enabled devices. Search and Agent/admission filters apply to the **entire loaded inventory**, not only one server page.
- The backend has no device text search and no reliable \`total\` in the list projection. \`DevicesView.vue\` loads consecutive 100-record pages up to a safety bound of 100 pages; if more devices exist, it displays an explicit error rather than silently claiming the result is complete.
- \`/devices/:deviceId\` fetches by immutable **hardware device_id** using \`devicesApi.get\` on each route-param change; URL refresh does not require the previous list state. Rendering includes HTTP loading/error/404 states.
- Delete device appears in the top-right overflow menu on Device Detail and shared Device Cards. There is no separate bottom Danger Zone. A confirmation dialog still precedes the revision-protected DELETE.
- Device Detail contains identity (with copyable ID, creation and update dates), related Agent and effective Template, admission switch, edit modal and confirmed delete. Agent and Template links are references, not additional permission grants.
- Edit supports changing name/description, Agent and optional linked Template override. Switching Agent clears the Template override to avoid using one that is not linked to the new Agent.
- Add device uses the existing claim-code flow: select an Agent on Devices Page, then use the existing \`ClaimDeviceEnrollmentModal\` to enter the 6-digit code. It does not require manual hardware-ID entry.
- The same UI card is shared by Agent Detail and Devices Page via \`components/devices/DeviceCard.vue\`; \`AgentDeviceCard.vue\` remains a compatible wrapper.
- The Device status field on the existing \`Device\` view model is legacy: \`online/offline\` is derived from DB \`enabled\`, **not WebSocket presence**. The new UI consistently labels it "Connection permitted" or "Connection disabled". Live presence is explicitly unavailable.

## API

Existing endpoints only, no new server API or schema:
- \`GET /api/admin/devices?page=N&page_size=100&sort=name\`
- \`GET /api/admin/devices/{device_id}\`
- \`PATCH /api/admin/devices/{device_id}\` with revision \`If-Match\`
- \`DELETE /api/admin/devices/{device_id}\` with revision \`If-Match\`
- \`POST /api/admin/device-enrollments/claim\`

\`AdminDevice\` includes \`created_at\` and \`updated_at\` as Unix seconds. Device ID is immutable: it is never in PATCH input. After updates the Detail page GETs the resource again to avoid stale data. Conflicting writes show the existing API error and permit refresh.

## Verification

Run in \`apps/admin-web\`:

\`\`\`sh
npm ci
npm run typecheck
npx vitest run src/components/devices/presentation.test.ts src/components/devices/DeviceCard.test.ts src/pages/devices/DeviceDetailPage.test.ts src/components/agents/AgentDeviceCard.test.ts src/components/agents/AgentDeviceList.test.ts
npm run build
\`\`\`

Acceptance tests:
1. Open \`/devices\` on mobile and desktop; observe cards, no horizontal table scrolling.
2. Search, filter by Agent/admission; count represents the complete fetched list.
3. Click card/keyboard Enter: opens Device Detail on the first try, including URL refresh.
4. Edit from list opens the target detail with edit modal.
5. Copy ID does not cause card navigation; Edit/Delete remain separate buttons.
6. Change Agent: override resets; only Templates linked to the new Agent are selectable.
7. Toggle admission: revision-protected PATCH then GET. No Online/Offline indicator from admission.
8. Delete: confirm, DELETE with revision, navigate back to list; conflicts/errors remain visible.
9. Add: select Agent, enter six-digit code; no manual ID required.

Known unrelated full-suite failures inherited from the feature base: \`ProviderCreateDrawer.test.ts\` and \`SpeakerEnrollmentWizard.test.ts\` assert outdated UI. The \`devices-ui\` workflow runs the scoped tests, typecheck and production build.
