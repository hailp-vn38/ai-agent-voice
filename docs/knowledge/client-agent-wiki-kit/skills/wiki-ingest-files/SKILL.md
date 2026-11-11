---
name: wiki-ingest-files
description: "Ingest nguồn thô trong sources/ thành trang wiki/ của một LLM Wiki vault bằng cách đọc/ghi file trực tiếp (không có tool wiki_* hay MCP knowledge), giữ đủ mọi ý của nguồn. Dùng khi được yêu cầu wiki-ingest một file, cả một folder markdown, nguồn còn tồn, hoặc ingest lại nguồn đã sửa."
---

# Wiki Ingest bằng file (`wiki-ingest-files`)

Bạn ingest bằng chính tool đọc/ghi file và shell của mình. Vault là thư mục chứa `SCHEMA.md`, `sources/` và `wiki/`; mọi đường dẫn dưới đây tính từ gốc vault, mọi lệnh chạy tại đó.

Trạng thái "đã ingest" **suy ra hoàn toàn từ file**: server đọc lại frontmatter và `[[link]]` khi đồng bộ. Vì vậy frontmatter và đường dẫn phải đúng từng ký tự theo mẫu ở mục [Mẫu trang](#mẫu-trang).

| Thư mục | Vai trò | Bạn được làm gì |
|---|---|---|
| `sources/` | nguồn thô, bất biến | chỉ đọc |
| `wiki/sources/`, `wiki/entities/`, `wiki/concepts/` | trang biên soạn | tạo, sửa |
| `wiki/index.md`, `wiki/log.md` | danh mục và nhật ký | chỉ thêm dòng theo bước 5 |
| `.wiki/`, `.obsidian/` | index và cấu hình sinh tự động | để nguyên |

## Bước 1 — Xác định nguồn cần ingest

Ba nhánh theo thứ người dùng đưa:

- **File nguồn cụ thể**: dùng đúng danh sách đó.
- **Một folder**: danh sách là mọi file `.md` trong folder, kể cả thư mục con (`find <folder> -name '*.md' | sort`). Folder nằm ngoài `sources/` thì trước hết chép nguyên trạng vào `sources/<domain>/<topic>/` bằng `cp -R`, giữ nguyên tên file và cấu trúc thư mục con; chọn `<domain>` đang có trong `sources/` trước khi đặt tên mới. Thứ tự ingest theo tên file; file tổng quan của folder (`index.md`, `README.md`) ingest sau cùng vì nó tóm lại các file kia.
- **Không nêu gì**: liệt kê nguồn tồn bằng lệnh dưới.

```bash
find sources -name '*.md' | sort | while read -r s; do
  w="wiki/$s"
  if [ ! -f "$w" ]; then
    grep -rqF --include='*.md' --exclude=index.md --exclude=log.md "[[${s%.md}" wiki || echo "NEW   $s"
  elif [ "$s" -nt "$w" ]; then echo "STALE $s"; fi
done
```

- `NEW`: chưa có trang tóm tắt và chưa trang wiki nào trích nguồn này.
- `STALE`: nguồn được sửa sau trang tóm tắt, ingest lại theo nhánh "trang đã tồn tại". Trong bản git clone mới, mtime không có nghĩa: so `git log -1 --format=%ct -- <file>` của hai file thay cho `-nt`.

Xong khi bạn có danh sách nguồn cụ thể và đã báo số file cho người dùng. Làm trọn bước 2–5 cho **một nguồn** rồi mới sang nguồn kế tiếp; mỗi nguồn được ingest sâu như thể nó là nguồn duy nhất, dù danh sách dài bao nhiêu. Khi mất ngữ cảnh giữa chừng, chạy lại lệnh trên để biết nguồn nào còn lại.

## Bước 2 — Đọc nhà và đọc nguồn

1. Đọc `SCHEMA.md` (thắng skill này khi mâu thuẫn) và `wiki/index.md`.
2. `find wiki -name '*.md' | sort` để biết các `<domain>` đang có và mọi slug trong `entities/`, `concepts/`.
3. Đọc **toàn bộ** file nguồn; file dài thì đọc nối tiếp từng đoạn tới dòng cuối.
4. Lập **bản kê ý** của nguồn: danh sách đánh số, đi tuần tự từ dòng đầu tới dòng cuối, mỗi mục một ý kèm số dòng trong nguồn. Một ý là một trong các thứ sau:
   - luận điểm, kết luận, khuyến nghị;
   - định nghĩa, quy tắc, điều kiện, bước trong quy trình;
   - con số, ngày tháng, tên riêng, đơn vị gắn với một nhận định;
   - ví dụ minh họa mang thông tin mới;
   - rủi ro, ngoại lệ, giới hạn, điều nguồn nói là chưa chắc;
   - khối code, công thức, bảng (mỗi khối một mục).

   Nguồn không có heading (báo cáo liền mạch, transcript) thì kê theo từng đoạn văn. Bản kê ý là thước đo ở bước 5: ý nào không vào bản kê thì sẽ mất khỏi wiki.
5. Lập bản phân tích:
   - `<domain>/<topic>` của nguồn = thư mục chứa nó (`sources/trading/fvg/x.md` → `trading/fvg`).
   - Ứng viên entity (vật có tên riêng: người, tổ chức, sản phẩm, công cụ, model) và concept (ý tưởng, phương pháp, chiến lược, mô hình). Chỉ **page-worthy** khi là trọng tâm của nguồn, lặp lại qua nhiều nguồn, hoặc mang luận điểm/số liệu đáng kể; nhắc thoáng qua thì nằm lại trong trang tóm tắt.
   - Với mỗi ứng viên page-worthy, tìm trang sẵn có: `find wiki/entities wiki/concepts -name '<slug>.md'` và `grep -rli '<tên hoặc viết tắt>' wiki/entities wiki/concepts`. Trùng slug hoặc trùng `aliases` nghĩa là trang đã tồn tại, dùng lại đúng đường dẫn đó.
   - Trang tóm tắt nguồn nào liên quan trực tiếp (cùng topic, cùng concept).
   - Chỗ nguồn này mâu thuẫn hoặc bổ sung giới hạn cho trang đang có.

Xong khi bản kê ý phủ tới dòng cuối của nguồn (mỗi heading và mỗi đoạn văn có ít nhất một mục), và mỗi ứng viên page-worthy đã có nhãn "tạo mới tại `<đường dẫn>`" hoặc "gộp vào `<đường dẫn>`".

## Bước 3 — Viết trang tóm tắt nguồn

Đường dẫn là ảnh gương của nguồn, giữ nguyên tên file: `sources/<rel>.md` → `wiki/sources/<rel>.md`. Viết theo [mẫu Source summary](#source-summary).

Trang này là bản **biên soạn lại toàn bộ** nguồn, không phải bản rút gọn: người chỉ đọc trang wiki phải nắm được mọi ý mà người đọc nguồn nắm được. Độ dài theo nguồn; nguồn dài thì trang dài. Chỉ được bỏ lời chào, quảng cáo, câu đệm và chỗ nguồn tự lặp lại.

- Viết lần lượt theo bản kê ý: mỗi mục thành một câu hoặc một gạch đầu dòng có đủ chủ thể, điều kiện và con số của nó. Gom mục vào heading theo chủ đề, giữ trình tự lập luận của nguồn.
- Giữ nguyên văn code, SQL, công thức, tham số cấu hình, bảng số liệu trong code fence hoặc bảng Markdown.
- Mỗi luận điểm, con số, giới hạn gắn đúng chủ thể mà nguồn gán cho nó.
- Lần đầu nhắc tới mỗi entity/concept page-worthy, viết thành `[[link]]` theo [Dạng link](#dạng-link). Mọi trang entity/concept bạn tạo ở bước 4 phải được trang tóm tắt này link tới.
- Văn xuôi và heading theo ngôn ngữ của các trang đang có trong vault; tên riêng, mã, định danh kỹ thuật, đường dẫn, URL giữ nguyên dạng gốc.
- Ảnh và video viết dạng nhúng `![mô tả](url)`.
- Sơ đồ quy trình (khi nguồn có): Mermaid `flowchart TD|LR` nhiều dòng, nhãn node trong nháy kép: `A["Vùng cân bằng"] --> B["Cú phá vỡ"]`.

Xong khi mọi mục của bản kê ý đã được viết vào trang.

## Bước 4 — Tạo hoặc gộp trang entity / concept

Đường dẫn: `wiki/entities/<domain>/<slug>.md`, `wiki/concepts/<domain>/<slug>.md`; công cụ/ngôn ngữ dùng chung nhiều domain đặt ở `wiki/entities/shared/`. `<domain>`, `<slug>` là `kebab-case` ASCII, tiếng Việt bỏ dấu (`khong-khi-lanh`). Slug là duy nhất trên toàn bộ `entities/` và `concepts/`.

**Trang chưa có** — viết theo [mẫu Entity / Concept](#entity--concept). 1–2 câu mở đầu tự đứng được: nó là gì, thuộc domain nào, dùng khi nào. `aliases` gồm viết tắt và tên gọi Việt/Anh.

**Trang đã có** — đọc trang, rồi sửa tại chỗ thành một bài thống nhất:

- Giữ mọi luận điểm cũ; thêm đóng góp của nguồn mới vào đúng mục.
- Giữ nguyên `title`, `category`, `domain` và đường dẫn. Hợp (union) `sources`, `aliases`, `tags`, cả ở top-level lẫn `qmd.metadata`. Đặt `updated_at` là thời điểm hiện tại.
- Thêm cặp `Source` + `Raw source` của nguồn mới vào mục `## Liên kết`.
- Nguồn mới mâu thuẫn với luận điểm cũ: giữ cả hai và thêm callout, trích cả hai nguồn:

  ```markdown
  > [!WARNING] Contradiction / Caveat
  > <điểm mâu thuẫn>, theo [[sources/a/b/x.md|x.md]] và [[sources/a/c/y.md|y.md]].
  ```

Sau đó, với mỗi trang tóm tắt liên quan trực tiếp tìm được ở bước 2, thêm vào mục link cuối trang đó một dòng trỏ về trang tóm tắt mới, và trang mới cũng link ngược lại.

Xong khi mọi ứng viên page-worthy của bước 2 có file tại đường dẫn đã gán nhãn, và file đó liệt kê nguồn này trong `sources:`.

## Bước 5 — Cập nhật index, log và kiểm tra

**`wiki/index.md`** — thêm (hoặc sửa) một dòng cho mỗi trang đã tạo/sửa, trong đúng section (`## Source Summaries`, `## Entities`, `## Concepts`), giữ thứ tự theo đường dẫn. Mỗi dòng là một dòng đơn:

```markdown
- [[wiki/<đường-dẫn-không-.md>|<tên-file.md>]] — <summary> *(sources: <số nguồn>, updated: <YYYY-MM-DD>, primary: [[<nguồn đầu tiên trong sources:>|<tên-file.md của trang>]])*
```

`<summary>`, số nguồn và ngày lấy đúng từ frontmatter của trang.

**`wiki/log.md`** — nối vào cuối file, giờ UTC lấy bằng `date -u '+%Y-%m-%d %H:%M'`, tiêu đề là đường dẫn nguồn bỏ tiền tố `sources/`:

```markdown
## [2026-11-08 11:23] ingest | trading/fvg/the-secret-fvg-breakaway-gap.md
- Source: `sources/trading/fvg/the-secret-fvg-breakaway-gap.md`
- Wiki page: `wiki/sources/trading/fvg/the-secret-fvg-breakaway-gap.md` (filename preserved: `the-secret-fvg-breakaway-gap.md`)
- Entities linked: <domain>/<slug>, ...
- Concepts linked: trading/breakaway-gap, trading/fair-value-gap

```

Bỏ dòng `Entities linked` / `Concepts linked` khi danh sách rỗng.

**Kiểm tra link** — chạy với mọi file bạn đã tạo hoặc sửa:

```bash
for f in <các file đã chạm>; do
  grep -o '\[\[[^]|#\\]*' "$f" | sed 's/^\[\[//' | sort -u | while read -r t; do
    case "$t" in sources/*|wiki/*) p="$t" ;; *) p="wiki/$t" ;; esac
    [ -f "$p" ] || [ -f "$p.md" ] || echo "BROKEN $f -> $t"
  done
done
```

**Kiểm tra độ phủ** — đối chiếu trang tóm tắt với nguồn:

1. Đọc lại trang tóm tắt vừa viết, đi qua từng mục của bản kê ý và chỉ ra heading của trang chứa nó. Mục nào không chỉ ra được thì viết bổ sung ngay.
2. Chạy lệnh dò con số và ngày tháng:

   ```bash
   s=sources/<rel>.md; w="wiki/$s"
   grep -oE '[0-9]+([.,:/-][0-9]+)*%?' "$s" | sort -u | while read -r n; do
     grep -qF -- "$n" "$w" || echo "MISSING $n"
   done
   ```

   Với mỗi dòng `MISSING`, mở nguồn tại chỗ có con số đó: nó thuộc một ý (số liệu, ngày, ngưỡng, tham số) thì thêm ý đó vào trang; nó là nhiễu (số thứ tự, mốc thời gian transcript, ID) thì bỏ qua.
3. Số khối code và số bảng trong trang không ít hơn trong nguồn (`grep -c '^```'`, `grep -c '^|'` trên hai file).

Nguồn này xong khi:

- mọi mục của bản kê ý có chỗ trong trang tóm tắt, và mọi dòng `MISSING` đã được thêm hoặc xếp vào nhiễu;
- lệnh kiểm tra link không in dòng `BROKEN` nào;
- chạy lại lệnh ở bước 1 không còn thấy nguồn này;
- mỗi file đã chạm bắt đầu bằng `---` ở dòng 1, và `qmd.metadata.tags` trùng khít `tags`;
- mỗi trang entity/concept mới có ít nhất một trang khác (ngoài `index.md`, `log.md`) link tới.

## Bước 6 — Khép lại cả lô

Sau nguồn cuối cùng của danh sách:

1. Đếm đối chiếu: số file `.md` dưới `sources/<folder>` bằng số file dưới `wiki/sources/<folder>`, và lệnh ở bước 1 không còn in nguồn nào thuộc danh sách.
2. Các trang tóm tắt cùng folder link tới nhau: mỗi trang có mục `Nguồn liên quan` trỏ tới các trang cùng folder chia sẻ concept với nó; trang của file tổng quan (`index.md`) link tới mọi trang trong folder.
3. Mỗi trang entity/concept được nhiều nguồn trong lô bổ sung đọc lên vẫn là một bài thống nhất, không phải các đoạn nối đuôi theo từng nguồn.
4. Báo cho người dùng một bảng, mỗi nguồn một dòng: số mục trong bản kê ý, số mục đã vào trang, số `MISSING` đã thêm / xếp vào nhiễu; kèm danh sách trang entity/concept tạo mới, trang gộp, và callout mâu thuẫn đã thêm.

Git commit/push là việc của người dùng, chỉ làm khi họ yêu cầu.

## Mẫu trang

### Dạng link

Link nội bộ luôn là `[[đường-dẫn|Nhãn]]` viết trần (ngoài backtick); trong ô bảng Markdown thì escape dấu gạch: `[[...\|Nhãn]]`.

| Đích | Dạng |
|---|---|
| entity | `[[entities/<domain>/<slug>\|Nhãn]]` |
| concept | `[[concepts/<domain>/<slug>\|Nhãn]]` |
| trang tóm tắt nguồn | `[[wiki/sources/<domain>/<topic>/<tên-file>\|Tiêu đề]]` (bỏ `.md`) |
| nguồn thô | `[[sources/<domain>/<topic>/<tên-file>.md\|<tên-file>.md]]` (giữ `.md`) |

### Tags

Ba facet, đúng thứ tự, `kebab-case` ASCII, không có `#`:

1. loại trang: `wiki/source` | `wiki/entity` | `wiki/concept`;
2. 1–3 tag `topic/<domain>/<topic>`: trang tóm tắt dùng thư mục nguồn của nó; dùng lại tag `topic/...` đang có trong vault (`grep -rhoE '"topic/[^"]+"' wiki | sort -u`) trước khi đặt tag mới;
3. 2–5 từ khóa chủ đề.

### Source summary

`updated_at` lấy bằng `date -u +%Y-%m-%dT%H:%M:%SZ`. `summary` là một câu, một dòng, tự đứng được; dấu `"` bên trong viết thành `\"`. Ingest lại trang đã tồn tại: giữ các mục `sources` cũ, thêm nguồn mới, `source_count` = số mục.

```markdown
---
title: "The Secret FVG that We Don't Trade Back Into (Breakaway Gap)"
category: "source_summary"
summary: "<một câu>"
source_file: "[[sources/trading/fvg/the-secret-fvg-breakaway-gap.md|the-secret-fvg-breakaway-gap.md]]"
source_count: 1
sources:
  - "[[sources/trading/fvg/the-secret-fvg-breakaway-gap.md|the-secret-fvg-breakaway-gap.md]]"
tags:
  - "wiki/source"
  - "topic/trading/fvg"
  - "breakaway-gap"
  - "market-structure"
updated_at: "2026-11-08T11:23:34Z"
qmd:
  metadata:
    category: "source_summary"
    source_file: "sources/trading/fvg/the-secret-fvg-breakaway-gap.md"
    source_count: 1
    sources:
      - "sources/trading/fvg/the-secret-fvg-breakaway-gap.md"
    tags:
      - "wiki/source"
      - "topic/trading/fvg"
      - "breakaway-gap"
      - "market-structure"
---

> **Raw Sources:** [[sources/trading/fvg/the-secret-fvg-breakaway-gap.md|the-secret-fvg-breakaway-gap.md]]  
> **Summary:** <cùng câu summary>

# <Title>

## Định vị và luận điểm cốt lõi
## 1. <Phần kỹ thuật / dữ kiện>
## 2. ...
## Bảng điều kiện / số liệu          (khi nguồn có)
## Điểm cốt lõi, rủi ro và giới hạn
## Nguồn và liên kết

- Nguồn gốc: [[sources/trading/fvg/the-secret-fvg-breakaway-gap.md|the-secret-fvg-breakaway-gap.md]]
- Khái niệm liên quan: [[concepts/trading/breakaway-gap|Breakaway Gap]]
- Nguồn liên quan: [[wiki/sources/trading/sharp-turn-entry/the-sharp-turn-entry-model|The Sharp Turn Entry Model]]
```

Dòng `> **Raw Sources:**` kết thúc bằng hai dấu cách.

### Entity / Concept

Entity dùng `category: "entity"` và tag `wiki/entity`; phần còn lại giống hệt.

```markdown
---
title: "Fair Value Gap"
category: "concept"
domain: "trading"
topic: "fair-value-gap"
summary: "<một câu>"
aliases:
  - "FVG"
  - "khoảng giá trị hợp lý"
sources:
  - "[[sources/trading/sharp-turn-entry/the-sharp-turn-entry-model.md|the-sharp-turn-entry-model.md]]"
tags:
  - "wiki/concept"
  - "topic/trading/fair-value-gap"
  - "fvg"
  - "price-action"
updated_at: "2026-11-03T10:00:00Z"
qmd:
  metadata:
    category: "concept"
    domain: "trading"
    topic: "fair-value-gap"
    tags:
      - "wiki/concept"
      - "topic/trading/fair-value-gap"
      - "fvg"
      - "price-action"
---

# Fair Value Gap

[[concepts/trading/fair-value-gap|Fair Value Gap (FVG)]] là ...

## <Các mục nội dung theo nguồn>

## Liên kết

- **Source:** [[wiki/sources/trading/sharp-turn-entry/the-sharp-turn-entry-model|The Sharp Turn Entry Model]]
- **Raw source:** [[sources/trading/sharp-turn-entry/the-sharp-turn-entry-model.md|the-sharp-turn-entry-model.md]]
- **Concept liên quan:** [[concepts/trading/sharp-turn-entry-model|Sharp Turn Entry Model]]
```

`topic` là topic riêng của trang, thường chính là slug. Mỗi nguồn trong `sources:` có một cặp `Source` + `Raw source` ở `## Liên kết`.
