# Voice Agent Studio (Admin Web)

Standalone management UI for `voice-agent-server`. The Vue application is outside Core V1 and must not own Voice Session state.

## Stack

- Vue 3 + Vite + TypeScript
- Vue Router
- Pinia
- Tailwind CSS 4
- shadcn-vue conventions with locally owned UI primitives
- Lucide Vue icons

## Requirements

- Node.js 22.12 or newer
- Rust server listening on `http://127.0.0.1:8000` for the default development proxy
- An Admin API bearer token, pasted into the connect form on first load

## Start

```bash
cd apps/admin-web
cp .env.example .env
npm install
npm run dev
```

Open `http://127.0.0.1:5173`.

Development requests for `/health`, `/ready`, `/mcp/vision/*`, `/voice` and `/api/admin/*` are proxied to
`VITE_DEV_PROXY_TARGET`.

## Authentication

The Admin API requires a bearer token. Paste it into the connect form at the top of the app; it is kept
in `sessionStorage` under `voice-agent-admin-token` and sent as `Authorization: Bearer <token>` on every
`/api/admin/*` request. Writes also send `If-Match: "<revision>"`, so a stale screen is detected instead
of silently overwriting someone else's change.

## Pages

### Overview — `/overview` (default `/`)

- Source-backed counts for Agents, Templates, Providers and registered Devices from `useAdminStore`.
- `GET /ready` and `GET /api/admin/system` for readiness, active sessions and provider runtime aggregates.
- Unknown values display an em dash; no fake device online count or voice telemetry.

### Devices — `/devices`

- Search registered devices and follow their Agent links; shows effective Template and administrative Enabled/Disabled.
- Device create/edit/delete remain on Agent Detail; live WS connection state is not available here.


### External MCP — `/mcp`

- Global MCP Server catalog with create/edit/enable/disable/delete and incremental loading.
- Streamable HTTP configuration and server-resolved SecretRef authentication; existing auth secrets are redacted in GET responses and are not prefilled.
- Filter/search operate on loaded catalog pages; Enabled is not a live connection status.
- Agent Detail → External Tools now includes MCP bindings (revisioned against Agent) and observed tool contract reviews. See [MCP Studio implementation guide](docs/mcp-studio-implementation-guide.md).

### Agents — `/agents`

- Agent Detail now uses the Studio / External Tools / Speakers / Devices tabs with a pipeline summary.

- Agent cards with the default template, provider chips, device count and add-device action.
- Create agent dialog.
- Click a card to open `/agents/:agentId`.
- Agent detail includes actions to add device, edit agent and delete agent.
- The template switcher lists only templates linked to this agent, and can create a new template, link an
  existing global one, set the default or jump to `/templates`.
- The AI Pipeline shows provider bindings split by VAD / ASR / LLM / TTS / Vision and is shared with the
  template detail page.
- Linked providers open a provider detail modal and support edit/unlink.
- Empty binding slots list existing providers of the matching type.
- Prompt and device list are visible on the same detail page. Device overrides can only point at a template
  linked to the agent.
- Devices support create, edit and delete.

### Templates — `/templates`

- Catalog of global, reusable AI configurations, shared by any number of agents.
- Search plus language and usage filters, with derived totals (templates, in use, unused, agent links).
- Compact cards show language, the provider pipeline summary and derived agent usage (count plus up to
  three agent names).
- Create, edit, copy (a new global template with a custom name) and delete. Deleting is refused while any agent links the template or a device
  overrides to it; duplicates start unlinked.
- Click a card to open `/templates/:templateId`, which reuses the AI Pipeline, shows the prompt and lists
  the agents using it as `Default` or `Linked`, with unlink and navigation to the agent detail.

### Providers — `/providers`

- Provider Catalog with search (name, adapter, model), a status filter and a tab per provider type
  (All, VAD, ASR, LLM, TTS, Vision) showing counts. The All view groups cards by type in pipeline
  order; a single-type view renders the grid without a redundant group heading.
- Provider card leads with the provider name, then a monospace adapter, model, description and derived
  template usage (`Used by N templates` plus a name preview), with `Test` and an overflow menu.
- Card body opens a side sheet so the catalog stays visible; it holds provider information, redacted
  endpoint, template usage and the provider test card. `Test` on a card opens it scrolled to the test
  section.
- Usage counts come from `AgentTemplate.providerBindings` through derived selectors, never from an
  agent edge, and nothing about usage is persisted on the provider.
- Overflow menu: view details, edit, link to template, duplicate, delete. Delete is the last,
  destructive item.
- Linking targets a template, never an agent. The provider role is derived from the provider type, and
  an occupied slot shows an explicit replace confirmation instead of replacing silently.
- Editing a shared provider and deleting one both state the blast radius. Deleting removes the
  provider and clears its binding from every template; templates, agents and devices are kept.
- Endpoints are redacted before display: credentials in userinfo or sensitive query parameters are
  masked.

### System — `/system`

- Rust server health, readiness and basic transport information.
- Management entity counts.
- Reload-from-server action.

## Redesign and connection contract

See [Voice Agent Studio redesign and API alignment](docs/voice-agent-studio-redesign.md). This guide records implemented P0/P1 components separately from planned Playground/Reports APIs. Playground, Reports, live device status and audio telemetry are not implemented by this UI foundation.

The theme defaults to dark for new visitors; the explicit saved light/dark preference remains authoritative. The existing Vue API clients, Pinia read model and Rust database ownership are unchanged.

## Languages

The management UI ships in English and Vietnamese. `src/i18n/messages.ts` owns the key set, so a missing
translation is a compile error. Switch the interface from the sidebar footer; the choice persists in
`localStorage` and `document.documentElement.lang` follows it.

A template's `language` field is data, not UI copy, so it is never translated. Its suggestions live in
`templateLanguageOptions` in `src/domain/admin.ts`.

## Data layer

`src/api/*` is the only module that talks HTTP. It owns the request wrapper (base URL, bearer token,
`If-Match`), the error envelope and one client per resource, plus the wire types under `src/api/types`.

`src/stores/admin.ts` is the read model the screens render from. `loadAll()` pulls agents, templates,
templates' provider bindings, providers and devices once, and every selector is a synchronous read of
that cache, so a page never awaits to draw a card. Mutations go back through `src/api/*` and then patch
the cache in place. Failures surface as a message in the banner at the top of the app instead of an
unhandled rejection, and a `revision_conflict` triggers a reload so the screen matches the server again.

`src/domain/admin.ts` holds the view models the components bind to. They are deliberately separate from
the wire types: the API has no `createdAt`/`updatedAt` and no last-seen timestamp, so the UI renders an
em dash for those fields.

The browser must not read or mutate `config.toml` directly.

## Build

```bash
npm run typecheck
npm run build
npm run preview
```

## shadcn-vue components

`components.json` is included. Additional primitives can be generated later in a network-enabled environment:

```bash
npx shadcn-vue@latest add table dialog dropdown-menu input select
```

The `tabs` primitive is locally owned under `src/components/ui/tabs`.
