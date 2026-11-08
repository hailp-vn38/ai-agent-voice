# Dùng lại UI Knowledge

Nguồn: `apps/admin-web/src/pages/knowledge/`, gồm `KnowledgePage.vue`, `KnowledgeNoteView.vue`, `KnowledgeGraph.vue`, `RightPanelTabs.vue`, `useKnowledgeWorkspace.ts`, `useDragReorder.ts`, `mermaidRenderer.ts`, `tradingChart*`, `knowledge-reader.css` và `src/lib/markdown.ts`.

## Seam: KnowledgeBackend

Hiện nay UI gọi HTTP `/api/admin/knowledge/*` từ nhiều chỗ. Cần gom các lệnh gọi đó về sau **một interface**:

```ts
export interface KnowledgeBackend {
  tree(scope: Scope): Promise<TreeNode>
  read(path: DocPath, window?: ReadWindow): Promise<Page>
  search(q: SearchQuery): Promise<Hit[]>
  graph(focus?: DocPath): Promise<Graph>
  status(): Promise<VaultStatus>
  apply(change: Change): Promise<Applied>
  onChange(cb: (e: VaultEvent) => void): () => void   // trả về hàm unsubscribe
}
```

Interface này giống hệt `Vault` ở [core.md](core.md). Một vốn từ dùng xuyên suốt từ Rust lên tới UI.

Đây là **seam thật** vì có hai adapter:

| Adapter | Dùng ở | Cài đặt |
|---|---|---|
| `HttpBackend` | `admin-web` | `fetch('/api/admin/knowledge/…')`. `onChange` dùng SSE/notifications sẵn có, hoặc polling |
| `TauriBackend` | `wiki-desktop` | `invoke('vault_*')`, `listen('vault://changed')` |

Thêm một adapter thứ ba là `MemoryBackend` (in-memory) cho test Vitest. Các test `KnowledgePage.test.ts` hiện đang mock `fetch` có thể chuyển sang dùng nó.

Backend được inject bằng `provide('knowledgeBackend', …)` ở root app. `useKnowledgeWorkspace` lấy nó qua `inject`. Component không import adapter cụ thể nào.

## Đóng gói code dùng chung

Có hai cách, nên chọn cách A:

- **A. Workspace package `packages/knowledge-ui`** (pnpm/npm workspaces). Chuyển các file ở trên vào đó. `admin-web` và `wiki-desktop` cùng phụ thuộc vào package này. Ranh giới rõ ràng và không thể lỡ import nhầm code admin.
- B. `wiki-desktop` import thẳng `../admin-web/src/pages/knowledge` qua alias của Vite. Làm nhanh được, nhưng sẽ kéo theo các phụ thuộc ngầm như i18n, `AdminShell`, router.

## Những gì cần tách hoặc bỏ

- **Wiki Chat** (`WikiChatPanel.vue`, `useWikiChat.ts`, `api/wiki-chat.ts`): v1 ẩn tab chat trong `RightPanelTabs` khi backend không có khả năng `chat`. Thêm cờ `capabilities: { chat: boolean }` vào `KnowledgeBackend`.
- **i18n:** chỉ lấy các key `knowledge.*` trong `i18n/messages.ts` để đưa vào package.
- **Router:** `KnowledgePage` không được phụ thuộc vào `vue-router` của admin. Trạng thái "trang đang mở" đưa vào `useKnowledgeWorkspace`. Mỗi app tự đồng bộ trạng thái này với URL hoặc window title.
- **Settings Knowledge** (`/knowledge/settings`): app desktop có Settings riêng (vault, MCP). Không dùng lại trang này.

## Cảm giác native trên macOS

- Cửa sổ có `titleBarStyle: "Overlay"`, sidebar dùng hiệu ứng vibrancy (`window-vibrancy` crate).
- Theo dõi `prefers-color-scheme`. CSS hiện tại đã có token sáng/tối thì giữ nguyên.
- Cho phép mở nhiều cửa sổ (`⌘⇧N`), mỗi cửa sổ là một vault hoặc một trang.
