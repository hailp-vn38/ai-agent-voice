use super::{format_wiki_markdown, resolve_wiki_scoped_path, wiki, wiki_error, *};
use llm_wiki::{
    CollectionScope, GetOptions, HybridQueryRequest, MultiGetOptions, SearchOptions, WikiEngine,
    slugify,
};
use serde::Deserialize;

#[derive(Default, Deserialize)]
pub(in crate::app::admin) struct SearchQuery {
    pub(super) q: Option<String>,
    pub(super) limit: Option<i64>,
    #[serde(default)]
    pub(super) space: Option<String>,
    #[serde(default)]
    pub(super) mode: Option<String>,
}

#[derive(Deserialize)]
pub(in crate::app::admin) struct PageLines {
    pub(super) from_line: Option<usize>,
    pub(super) max_lines: Option<usize>,
}

#[derive(Clone, Copy)]
enum BrowserSpaceParam {
    Wiki,
    Raw,
}

impl BrowserSpaceParam {
    fn parse(raw: Option<&str>) -> Option<Self> {
        match raw.map(str::trim).filter(|s| !s.is_empty()) {
            None | Some("wiki") => Some(Self::Wiki),
            Some("raw") => Some(Self::Raw),
            Some(_) => None,
        }
    }

    fn from_collection(collection: CollectionScope) -> Self {
        if collection == CollectionScope::Sources {
            Self::Raw
        } else {
            Self::Wiki
        }
    }

    fn as_str(self) -> &'static str {
        match self {
            Self::Wiki => "wiki",
            Self::Raw => "raw",
        }
    }

    fn collection_scope(self) -> CollectionScope {
        match self {
            Self::Wiki => CollectionScope::Wiki,
            Self::Raw => CollectionScope::Sources,
        }
    }

    fn list_glob(self) -> &'static str {
        match self {
            Self::Wiki => "wiki/**/*.md",
            Self::Raw => "sources/**/*.md",
        }
    }

    fn ui_path(self, rel_path: &str) -> String {
        match self {
            Self::Wiki => rel_path.to_owned(),
            Self::Raw => rel_path.replacen("sources/", "raw/", 1),
        }
    }
}

#[derive(Clone, Copy)]
enum SearchModeParam {
    Hybrid,
    Lex,
    Vec,
}

impl SearchModeParam {
    fn parse(raw: Option<&str>) -> Option<Self> {
        match raw.map(str::trim).filter(|s| !s.is_empty()) {
            None | Some("hybrid") => Some(Self::Hybrid),
            Some("lex") => Some(Self::Lex),
            Some("vec") => Some(Self::Vec),
            Some(_) => None,
        }
    }

    fn execute(
        self,
        engine: &WikiEngine,
        query: &str,
        space: BrowserSpaceParam,
        limit: usize,
    ) -> llm_wiki::WikiResult<Vec<llm_wiki::SearchResultItem>> {
        let collections = vec![space.collection_scope()];
        match self {
            Self::Hybrid => engine.query(HybridQueryRequest {
                query: Some(query.to_owned()),
                collections,
                limit,
                ..Default::default()
            }),
            Self::Lex => engine.search_lex(
                query,
                SearchOptions {
                    collections,
                    filter: None,
                    limit,
                    min_score: 0.0,
                },
            ),
            Self::Vec => engine.search_vec(
                query,
                SearchOptions {
                    collections,
                    filter: None,
                    limit,
                    min_score: 0.0,
                },
            ),
        }
    }
}

fn base_page_item_json(
    collection: CollectionScope,
    rel_path: &str,
    docid: &str,
    title: &str,
    source_ref: impl Into<Value>,
    excerpt: &str,
) -> Value {
    let space = BrowserSpaceParam::from_collection(collection);
    serde_json::json!({
        "id": rel_path,
        "docid": docid,
        "space": space.as_str(),
        "path": space.ui_path(rel_path),
        "title": title,
        "revision": 1,
        "status": "verified",
        "source_ref": source_ref.into(),
        "excerpt": excerpt,
        "relative_path": rel_path,
    })
}

pub(in crate::app::admin) async fn knowledge_search(
    State(state): State<AppState>,
    Query(q): Query<SearchQuery>,
    req: Request,
) -> Response {
    let engine = match wiki(&state, &req) {
        Ok(engine) => engine,
        Err(resp) => return resp,
    };
    let Some(space) = BrowserSpaceParam::parse(q.space.as_deref()) else {
        return error(&req, StatusCode::BAD_REQUEST, "invalid_arguments");
    };
    let Some(mode) = SearchModeParam::parse(q.mode.as_deref()) else {
        return error(&req, StatusCode::BAD_REQUEST, "invalid_arguments");
    };
    let limit = q.limit.unwrap_or(50).clamp(1, 200) as usize;
    if let Some(query) =
        q.q.as_deref()
            .map(str::trim)
            .filter(|query| !query.is_empty())
    {
        match mode.execute(&engine, query, space, limit) {
            Ok(results) => {
                let items: Vec<Value> = results
                    .into_iter()
                    .map(|item| {
                        let mut json = base_page_item_json(
                            item.collection,
                            &item.rel_path,
                            &item.docid,
                            &item.title,
                            item.virtual_uri,
                            &item.snippet,
                        );
                        if let Some(obj) = json.as_object_mut() {
                            obj.insert("score".to_owned(), serde_json::json!(item.score));
                            obj.insert("match_line".to_owned(), serde_json::json!(item.match_line));
                            obj.insert("context".to_owned(), serde_json::json!(item.context));
                        }
                        json
                    })
                    .collect();
                Json(serde_json::json!({
                    "items": items,
                    "next_cursor": Value::Null,
                }))
                .into_response()
            }
            Err(err) => wiki_error(&req, err),
        }
    } else {
        match engine.multi_get(space.list_glob(), MultiGetOptions::default()) {
            Ok(batch) => {
                let items: Vec<Value> = batch
                    .docs
                    .into_iter()
                    .take(limit)
                    .map(|doc| {
                        let excerpt: String = doc.content.chars().take(240).collect();
                        base_page_item_json(
                            doc.info.collection,
                            &doc.info.rel_path,
                            &doc.info.docid,
                            &doc.info.title,
                            doc.info.source_rel_path,
                            &excerpt,
                        )
                    })
                    .collect();
                Json(serde_json::json!({
                    "items": items,
                    "next_cursor": Value::Null,
                }))
                .into_response()
            }
            Err(err) => wiki_error(&req, err),
        }
    }
}

pub(in crate::app::admin) async fn knowledge_read(
    State(state): State<AppState>,
    Path(id): Path<String>,
    Query(lines): Query<PageLines>,
    req: Request,
) -> Response {
    let engine = match wiki(&state, &req) {
        Ok(engine) => engine,
        Err(resp) => return resp,
    };
    let opts = GetOptions {
        from_line: lines.from_line,
        max_lines: lines.max_lines,
        line_numbers: false,
        include_frontmatter: true,
    };
    match engine.get_document(&id, opts) {
        Ok(doc) => Json(serde_json::json!({
            "id": doc.info.rel_path,
            "docid": doc.info.docid,
            "title": doc.info.title,
            "body": doc.content,
            "content": doc.content,
            "revision": 1,
            "status": "verified",
            "source_ref": doc.info.source_rel_path,
            "relative_path": doc.info.rel_path,
            "virtual_uri": doc.virtual_uri,
            "tags": doc.info.tags,
            "metadata": doc.info.metadata,
        }))
        .into_response(),
        Err(err) => wiki_error(&req, err),
    }
}

#[derive(Deserialize)]
struct WikiWritePayload {
    title: String,
    body: String,
    #[serde(default)]
    source_ref: Option<String>,
    #[serde(default)]
    folder_path: Option<String>,
    #[serde(default)]
    file_name: Option<String>,
}

pub(in crate::app::admin) async fn knowledge_create(
    State(state): State<AppState>,
    req: Request,
) -> Response {
    let (req, input): (_, WikiWritePayload) = match json(req).await {
        Ok(v) => v,
        Err(resp) => return resp,
    };
    let engine = match wiki(&state, &req) {
        Ok(engine) => engine,
        Err(resp) => return resp,
    };
    if input.title.trim().is_empty() {
        return error(&req, StatusCode::BAD_REQUEST, "invalid_arguments");
    }
    let file_stem = input
        .file_name
        .as_deref()
        .filter(|s| !s.trim().is_empty())
        .map(|s| slugify(s, 64))
        .unwrap_or_else(|| slugify(&input.title, 64));
    let folder = input
        .folder_path
        .as_deref()
        .map(|f| f.trim().trim_matches('/').trim_start_matches("wiki/"))
        .filter(|f| !f.is_empty())
        .unwrap_or("concepts");
    let rel_path = format!("{folder}/{file_stem}.md");
    let markdown = format_wiki_markdown(&input.title, &input.body, input.source_ref.as_deref());
    match engine.write_wiki_file(&rel_path, &markdown) {
        Ok(info) => {
            let _ = engine.rebuild_or_sync_index_md();
            (
                StatusCode::CREATED,
                Json(serde_json::json!({
                    "id": info.rel_path,
                    "docid": info.docid,
                    "title": info.title,
                    "body": input.body,
                    "revision": 1,
                    "status": "verified",
                    "source_ref": info.source_rel_path,
                    "relative_path": info.rel_path,
                })),
            )
                .into_response()
        }
        Err(err) => wiki_error(&req, err),
    }
}

pub(in crate::app::admin) async fn knowledge_update(
    State(state): State<AppState>,
    Path(id): Path<String>,
    req: Request,
) -> Response {
    let (req, input): (_, WikiWritePayload) = match json(req).await {
        Ok(v) => v,
        Err(resp) => return resp,
    };
    let engine = match wiki(&state, &req) {
        Ok(engine) => engine,
        Err(resp) => return resp,
    };
    let target_rel = resolve_wiki_scoped_path(&engine, &id);
    let markdown = format_wiki_markdown(&input.title, &input.body, input.source_ref.as_deref());
    match engine.write_wiki_file(&target_rel, &markdown) {
        Ok(info) => {
            let _ = engine.rebuild_or_sync_index_md();
            Json(serde_json::json!({
                "id": info.rel_path,
                "docid": info.docid,
                "title": info.title,
                "body": input.body,
                "revision": 1,
                "status": "verified",
                "source_ref": info.source_rel_path,
                "relative_path": info.rel_path,
            }))
            .into_response()
        }
        Err(err) => wiki_error(&req, err),
    }
}

pub(in crate::app::admin) async fn knowledge_publish(
    State(state): State<AppState>,
    Path(id): Path<String>,
    req: Request,
) -> Response {
    let engine = match wiki(&state, &req) {
        Ok(engine) => engine,
        Err(resp) => return resp,
    };
    match engine.get_document(&id, GetOptions::default()) {
        Ok(doc) => {
            let _ = engine.rebuild_or_sync_index_md();
            Json(serde_json::json!({
                "id": doc.info.rel_path,
                "docid": doc.info.docid,
                "title": doc.info.title,
                "body": doc.content,
                "revision": 1,
                "status": "verified",
                "source_ref": doc.info.source_rel_path,
                "relative_path": doc.info.rel_path,
            }))
            .into_response()
        }
        Err(err) => wiki_error(&req, err),
    }
}
