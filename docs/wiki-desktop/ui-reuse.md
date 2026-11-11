# Dùng lại UI Knowledge

UI dùng chung nằm trong workspace package `packages/knowledge-ui`. Admin Web phụ thuộc package này qua npm workspace; app desktop có thể dùng cùng `KnowledgePage`, `KnowledgeGraph`, `KnowledgeNoteView`, editor, workspace composable, renderer Markdown/Mermaid/chart, CSS reader và backend trong bộ nhớ.

## Seam: KnowledgeBackend

`KnowledgeBackend` là hợp đồng dữ liệu của UI, dùng chung với `Vault` trong [core.md](core.md):

```ts
export interface KnowledgeBackend {
  readonly capabilities: { chat: boolean }
  tree(scope: Scope): Promise<VaultTreeNode>
  read(path: DocPath, window?: ReadWindow): Promise<VaultPage>
  search(query: string, options?: VaultSearchOptions): Promise<VaultHit[]>
  graph(focus?: DocPath): Promise<VaultGraph>
  status(): Promise<VaultStatus>
  apply(change: VaultChange): Promise<Applied>
  onChange(listener: (event: VaultEvent) => void): () => void
}
```

`MemoryKnowledgeBackend` ở trong package để chạy contract và UI tests. `HttpKnowledgeBackend` ở lại Admin Web; desktop sẽ cung cấp adapter Tauri trong ticket 08.

Backend, hàm dịch, theme, navigation và chart data được cung cấp qua injection keys của package. UI package không import API, router, shell, store hay component từ Admin Web. `useKnowledgeWorkspace` giữ trạng thái pane/tab của mỗi instance; app host có thể đồng bộ navigation với URL hoặc window title.

## Phạm vi giao diện

- Wiki Chat chỉ hiện khi `capabilities.chat` bật. Phần chat và conversation API hiện thuộc Admin Web; desktop v1 tắt khả năng này.
- Catalog `knowledgeV3.*` và `knowledgeEditor.*` nằm trong package, Admin Web ghép chúng vào catalog chính của mình.
- `/knowledge/settings` vẫn là trang riêng của Admin; desktop quản lý vault và MCP ở shell riêng.
