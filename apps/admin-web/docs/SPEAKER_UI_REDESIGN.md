# Speaker UI redesign — implementation & QA

## Scope

- \`/speakers\`: replace the oversized-key table with responsive Speaker cards (1 column mobile, 2 tablet, 3 wide desktop); preserve server paging, search, quick/validated enrollment, profile creation and delete.
- \`/speakers/:speakerKey\`: make the profile the primary content, show voiceprint status without confusing voice matching with identity verification, put technical fields under \`<details>\`, and keep edit, voice enrollment/purge and delete.
- The entire card opens \`speaker-detail\` via \`RouterLink\`. Edit/Delete are **sibling buttons above the full-card link**, not nested inside it. Edit navigates to \`/speakers/:speakerKey?edit=1\` and opens the editor after loading the speaker.
- Direct links and route-param-only navigation both GET the requested profile. Requests are aborted when a newer load starts or the page unmounts.

## API contract

No backend/database/API changes are required.

- \`GET /api/admin/speakers?page=...&page_size=...&sort=updated_at\` provides list cards. This backend uses **\`updated_at\` for descending** order (and \`-updated_at\` for ascending): do not reverse it casually.
- List entries expose \`enabled\` but **do not contain voiceprints or enrollment status**. The card badge therefore reports the administrative enabled/disabled flag, **not** an invented "Enrolled"/"Verified" state.
- \`GET /api/admin/speakers/:speakerKey\` provides \`voiceprints\` and \`enrollment_drafts\`. Detail status reflects this full projection. Neither a stored voiceprint nor holdout validation is identity authentication.
- \`PATCH /api/admin/speakers/:speakerKey\` presently returns the profile with **empty voiceprints/drafts arrays** regardless of stored relations. After PATCH, the Detail page **must GET again** to prevent a false "not enrolled" display.
- Mutations retain the API's revision-based \`If-Match\` preconditions; permission/constraint failures surface in the page error state. Speaker deletion may be rejected while voiceprints, active drafts or Agent bindings still reference it.
- Only the technical accordion displays the full speaker key and revision; keys remain available as search terms.

## Validation checklist

1. Open Speakers in desktop/mobile sizes: no clipped key/date columns, card names and descriptions wrap/truncate gracefully, both themes readable.
2. Click any card background, title or badge: navigate on the **first** click. Tab to the full-card link and press Enter: same result.
3. Click Edit and Delete on a card: neither action triggers a competing navigation. Edit opens the correct profile's editor; Delete asks for confirmation and reports server conflicts.
4. Open \`/speakers/:speakerKey\` directly and refresh the browser: correct profile loaded without dependence on the list state.
5. Navigate between two speaker keys using the same Detail route instance: the second record loads, with no flash of the old record.
6. Save an edited speaker already containing voiceprints: Detail must re-fetch and still show the stored voice status.
7. Verify empty/loading/API-error states, paging, search, quick enrollment and validated enrollment remain usable.
8. Check technical fields do not overflow the Detail layout; copy speaker key works in a secure browser context.

## Frontend verification commands

Run in \`apps/admin-web\`:

\`\`\`sh
npm ci
npm run typecheck
npm test
npm run build
\`\`\`

The repository's existing GitHub Actions workflow focuses on Rust; it does not currently run the admin-web checks.
