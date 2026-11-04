use std::collections::HashSet;
use std::sync::Arc;

use async_trait::async_trait;
use orchestrator_agent_rs::{
    HostTool, ModelMessage, ModelRole, ModelToolCall, ToolContext, ToolDescriptor, ToolError,
    ToolOutcome,
};
use orchestrator_flow_rs::{
    RecipeDecision, RecipeMetadata, RecipeProgress, RecipeStepKind, RecipeStepOutline,
    RecipeValidationError, TaskRecipe, validate_recipe_params,
};
use serde_json::{Value, json};

pub const PLAYLIST_RECIPE_ID: &str = "playlist_sources";
pub const PLAYLIST_RECIPE_ROOT: &str = "recipe_playlist_sources";
pub const PLAYLIST_DOMAIN_SELECTOR: &str = "recipe_playlist_domain_selector";
pub const PLAYLIST_COMPLETION_TOOL: &str = "recipe_topic_selection_complete";
const BATCH_SIZE: usize = 10;
const MAX_VIDEOS: usize = 50;
/// Schema value of the `domain` enum meaning "let the selector Agent Step choose". It exists only
/// on the wire; [`PlaylistParams::domain`] turns it into `None`.
const AGENT_AUTO_DOMAIN: &str = "agent tự chọn";

/// Live Wiki domain catalog, read on every use so domains created after startup are accepted.
pub type WikiDomainSource = Arc<dyn Fn() -> Vec<String> + Send + Sync>;

/// Typed view of the recipe parameters, parsed once from the validated JSON.
#[derive(Debug, Clone, PartialEq, Eq)]
struct PlaylistParams {
    playlist_url: Option<String>,
    video_urls: Vec<String>,
    /// `None` lets the selector choose among the current Wiki domains.
    domain: Option<String>,
    max_videos: usize,
    ingest_to_wiki: bool,
}

impl PlaylistParams {
    fn from_value(params: &Value) -> Self {
        Self {
            playlist_url: params.get("url").and_then(Value::as_str).map(str::to_owned),
            video_urls: params
                .get("video_urls")
                .and_then(Value::as_array)
                .into_iter()
                .flatten()
                .filter_map(Value::as_str)
                .map(str::to_owned)
                .collect(),
            domain: params
                .get("domain")
                .and_then(Value::as_str)
                .map(str::trim)
                .filter(|domain| !domain.is_empty() && *domain != AGENT_AUTO_DOMAIN)
                .map(str::to_owned),
            max_videos: params
                .get("max_videos")
                .and_then(Value::as_u64)
                .map_or(MAX_VIDEOS, |max| (max as usize).min(MAX_VIDEOS)),
            ingest_to_wiki: params
                .get("ingest_to_wiki")
                .and_then(Value::as_bool)
                .unwrap_or(false),
        }
    }

    fn source_urls(&self) -> Vec<String> {
        if let Some(url) = &self.playlist_url {
            return vec![url.clone()];
        }
        self.video_urls
            .iter()
            .take(self.max_videos)
            .cloned()
            .collect()
    }

    fn title(&self) -> String {
        let video_urls = self.video_urls.len().min(self.max_videos);
        let source = if video_urls > 0 {
            format!("{video_urls} video lẻ")
        } else {
            format!("playlist · tối đa {} video", self.max_videos)
        };
        let mut title = format!("Thu thập {source}");
        if let Some(domain) = &self.domain {
            title.push_str(" · ");
            title.extend(domain.chars().take(32));
        }
        if self.ingest_to_wiki {
            title.push_str(" · Wiki");
        }
        title
    }
}

#[derive(Debug, Clone, PartialEq, Eq, serde::Deserialize, serde::Serialize)]
#[serde(deny_unknown_fields)]
struct TopicSelection {
    domain: String,
    topic: String,
}

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
struct TopicSelectionDiagnosis {
    code: &'static str,
    errors: Vec<String>,
    expected: &'static str,
}

fn validate_topic_selection(selection: &TopicSelection) -> Vec<String> {
    let mut errors = Vec::new();
    if selection.domain.trim().is_empty() {
        errors.push("domain must be a non-empty Wiki domain".into());
    }
    if selection.topic.is_empty()
        || !selection
            .topic
            .bytes()
            .all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit() || byte == b'-')
        || selection.topic.starts_with('-')
        || selection.topic.ends_with('-')
        || selection.topic.contains("--")
    {
        errors.push("topic must be a lowercase kebab-case slug".into());
    }
    errors
}

/// First server-owned recipe. It lists a playlist or individual video URLs, fetches bounded
/// transcript batches, then saves successful items through the ordinary mutating HostTool pipeline.
pub struct PlaylistSourcesRecipe {
    wiki_domains: WikiDomainSource,
}

impl Default for PlaylistSourcesRecipe {
    fn default() -> Self {
        Self::new(Vec::new())
    }
}

impl PlaylistSourcesRecipe {
    /// Fixed domain catalog, for tests and callers without a live Wiki.
    pub fn new(wiki_domains: Vec<String>) -> Self {
        Self::with_domain_source(Arc::new(move || wiki_domains.clone()))
    }

    pub fn with_domain_source(wiki_domains: WikiDomainSource) -> Self {
        Self { wiki_domains }
    }

    fn wiki_domains(&self) -> Vec<String> {
        let mut wiki_domains = (self.wiki_domains)()
            .into_iter()
            .map(|domain| domain.trim().to_owned())
            .filter(|domain| !domain.is_empty() && domain != AGENT_AUTO_DOMAIN)
            .collect::<Vec<_>>();
        wiki_domains.sort();
        wiki_domains.dedup();
        wiki_domains
    }

    fn params_schema(&self) -> Value {
        let mut domains = vec![Value::String(AGENT_AUTO_DOMAIN.into())];
        domains.extend(self.wiki_domains().into_iter().map(Value::String));
        json!({
            "type": "object",
            "properties": {
                "url": {"type": "string", "format": "uri", "title": "Playlist URL"},
                "video_urls": {
                    "type": "array",
                    "title": "Danh sách URL video",
                    "items": {"type": "string", "format": "uri"}
                },
                "domain": {
                    "type": "string",
                    "title": "Wiki domain",
                    "default": AGENT_AUTO_DOMAIN,
                    "enum": domains,
                    "x-options-source": "wiki_domains"
                },
                "max_videos": {"type": "integer", "minimum": 1, "maximum": MAX_VIDEOS, "default": MAX_VIDEOS, "title": "Số video tối đa"},
                "ingest_to_wiki": {"type": "boolean", "default": false, "title": "Ingest vào Wiki sau khi thu thập"}
            },
            "required": []
        })
    }

    fn valid_youtube_url(raw: &str) -> bool {
        let Ok(url) = url::Url::parse(raw) else {
            return false;
        };
        if !matches!(url.scheme(), "http" | "https") {
            return false;
        }
        url.host_str().is_some_and(|host| {
            host == "youtu.be"
                || host == "www.youtu.be"
                || host == "youtube.com"
                || host.ends_with(".youtube.com")
        })
    }

    fn list_result<'a>(transcript: &'a [ModelMessage], url: &str) -> Option<&'a ModelMessage> {
        let (result, has_tool_call) =
            Self::result_for_tool_call(transcript, "yt_list_playlist", "url", url);
        result.or_else(|| {
            (!has_tool_call)
                .then(|| Self::latest_list(transcript))
                .flatten()
        })
    }

    fn status_for<'a>(transcript: &'a [ModelMessage], url: &str) -> Option<&'a ModelMessage> {
        let (result, has_tool_call) =
            Self::result_for_tool_call(transcript, "collection_status", "playlist_url", url);
        result.or_else(|| {
            (!has_tool_call)
                .then(|| Self::latest_status(transcript).map(|(_, status)| status))
                .flatten()
        })
    }

    fn result_for_tool_call<'a>(
        transcript: &'a [ModelMessage],
        tool_name: &str,
        argument_name: &str,
        argument_value: &str,
    ) -> (Option<&'a ModelMessage>, bool) {
        let mut has_tool_call = false;
        let result = transcript
            .iter()
            .flat_map(|message| message.tool_calls.iter())
            .filter(|call| {
                if call.name != tool_name {
                    return false;
                }
                has_tool_call = true;
                call.arguments.get(argument_name).and_then(Value::as_str) == Some(argument_value)
            })
            .filter_map(|call| Self::result_for(transcript, &call.id))
            .last();
        (result, has_tool_call)
    }

    fn status_items(params: &PlaylistParams, transcript: &[ModelMessage]) -> Vec<(String, String)> {
        let limit = params.max_videos;
        let mut items = Vec::new();
        for url in params.source_urls() {
            let Some(status) = Self::status_for(transcript, &url) else {
                continue;
            };
            items.extend(
                Self::next_items(&status.text, MAX_VIDEOS)
                    .into_iter()
                    .take(limit.saturating_sub(items.len())),
            );
            if items.len() >= limit {
                break;
            }
        }
        items
    }

    fn result_for<'a>(transcript: &'a [ModelMessage], call_id: &str) -> Option<&'a ModelMessage> {
        transcript.iter().find(|message| {
            message.role == ModelRole::Tool && message.tool_call_id.as_deref() == Some(call_id)
        })
    }

    fn calls<'a>(
        transcript: &'a [ModelMessage],
        name: &str,
    ) -> Vec<(&'a ModelToolCall, &'a ModelMessage)> {
        transcript
            .iter()
            .flat_map(|message| message.tool_calls.iter())
            .filter(|call| call.name == name)
            .filter_map(|call| Self::result_for(transcript, &call.id).map(|result| (call, result)))
            .collect()
    }

    fn successful_ids(transcript: &[ModelMessage], tool: &str) -> HashSet<String> {
        Self::calls(transcript, tool)
            .into_iter()
            .filter(|(_, result)| !result.text.trim_start().starts_with("error "))
            .filter_map(|(call, _)| {
                call.arguments
                    .get("video_id")
                    .and_then(Value::as_str)
                    .map(str::to_owned)
            })
            .collect()
    }

    fn latest_result_index(transcript: &[ModelMessage], tool_names: &[&str]) -> Option<usize> {
        transcript
            .iter()
            .enumerate()
            .filter_map(|(index, result)| {
                if result.role != ModelRole::Tool
                    || !tool_names.contains(&result.tool_name.as_deref()?)
                {
                    return None;
                }
                Some(index)
            })
            .max()
    }

    fn latest_status(transcript: &[ModelMessage]) -> Option<(usize, &ModelMessage)> {
        transcript.iter().enumerate().rev().find(|(_, message)| {
            message.role == ModelRole::Tool
                && message.tool_name.as_deref() == Some("collection_status")
        })
    }

    fn latest_list(transcript: &[ModelMessage]) -> Option<&ModelMessage> {
        transcript.iter().rev().find(|message| {
            message.role == ModelRole::Tool
                && message.tool_name.as_deref() == Some("yt_list_playlist")
        })
    }

    fn next_items(status: &str, max_videos: usize) -> Vec<(String, String)> {
        let Some((_, body)) = status.split_once("NEXT:") else {
            return Vec::new();
        };
        body.lines()
            .filter_map(|line| {
                let position = line.trim().split('.').next()?.parse::<usize>().ok()?;
                if position > max_videos {
                    return None;
                }
                let id = line.split("video_id=").nth(1)?.split_whitespace().next()?;
                let title = line.split("title=").nth(1).unwrap_or(id).trim().to_owned();
                Some((id.to_owned(), title))
            })
            .collect()
    }

    fn failed_source_list<'a>(
        params: &PlaylistParams,
        transcript: &'a [ModelMessage],
    ) -> Option<&'a ModelMessage> {
        params.source_urls().iter().find_map(|url| {
            Self::list_result(transcript, url)
                .filter(|result| result.text.trim_start().starts_with("error "))
        })
    }

    fn decision_failure(result: &ModelMessage, step: usize) -> RecipeDecision {
        RecipeDecision::Failed {
            stop_reason: result
                .text
                .lines()
                .next()
                .unwrap_or("recipe tool failed")
                .to_owned(),
            failed_step: step,
        }
    }

    fn selector_results(transcript: &[ModelMessage]) -> Vec<(&ModelToolCall, &ModelMessage)> {
        Self::calls(transcript, "subagent")
            .into_iter()
            .filter(|(call, _)| {
                call.arguments.get("agent").and_then(Value::as_str)
                    == Some(PLAYLIST_DOMAIN_SELECTOR)
            })
            .collect()
    }

    fn selector_output(result: &ModelMessage) -> Result<TopicSelection, Vec<String>> {
        let text = result.text.trim();
        if !text.starts_with("<subagent ") || !text.ends_with("</subagent>") {
            return Err(vec![
                "subagent result must contain only the completion envelope".into(),
            ]);
        }
        let Some(open_end) = text.find('>') else {
            return Err(vec![
                "subagent result is missing its completion envelope".into(),
            ]);
        };
        let close_start = text.len() - "</subagent>".len();
        let payload = text[open_end + 1..close_start].trim();
        let selection: TopicSelection = serde_json::from_str(payload).map_err(|error| {
            vec![format!(
                "completion output must match the declared JSON schema: {error}"
            )]
        })?;
        let errors = validate_topic_selection(&selection);
        if !errors.is_empty() {
            return Err(errors);
        }
        Ok(selection)
    }

    fn selector_decision(
        &self,
        params: &PlaylistParams,
        transcript: &[ModelMessage],
    ) -> Result<TopicSelection, RecipeDecision> {
        let attempts = Self::selector_results(transcript);
        if let Some((_, result)) = attempts.last()
            && !result.text.trim_start().starts_with("error ")
        {
            match Self::selector_output(result) {
                Ok(selection) => {
                    match params.domain.as_deref() {
                        Some(requested) if selection.domain != requested => {
                            return Err(RecipeDecision::Failed {
                                    stop_reason: "RECIPE_STEP_FAILED: agent selected a different domain than requested".into(),
                                    failed_step: 2,
                                });
                        }
                        Some(_) => {}
                        None => {
                            let wiki_domains = self.wiki_domains();
                            if !wiki_domains.is_empty() && !wiki_domains.contains(&selection.domain)
                            {
                                return Err(RecipeDecision::Failed {
                                        stop_reason: "RECIPE_STEP_FAILED: agent selected a domain outside the current Wiki catalog".into(),
                                        failed_step: 2,
                                    });
                            }
                        }
                    }
                    return Ok(selection);
                }
                Err(errors) if attempts.len() >= 2 => {
                    let diagnosis = TopicSelectionDiagnosis {
                        code: "INVALID_AGENT_STEP_OUTPUT",
                        errors,
                        expected: "JSON object with exactly string fields domain and topic; topic is a lowercase kebab-case slug",
                    };
                    return Err(RecipeDecision::Failed {
                        stop_reason: format!(
                            "RECIPE_STEP_FAILED: {}",
                            serde_json::to_string(&diagnosis).unwrap_or_default()
                        ),
                        failed_step: 2,
                    });
                }
                Err(errors) => {
                    return Err(self.selector_call(
                        params,
                        attempts.last().map(|(_, result)| *result),
                        Some(errors),
                    ));
                }
            }
        }
        if attempts.is_empty() {
            return Err(self.selector_call(params, None, None));
        }
        Err(RecipeDecision::Failed {
            stop_reason:
                "RECIPE_STEP_FAILED: classifier sub-agent did not return a completion result".into(),
            failed_step: 2,
        })
    }

    fn selector_failure(
        &self,
        params: &PlaylistParams,
        transcript: &[ModelMessage],
    ) -> Option<(usize, String)> {
        match self.selector_decision(params, transcript) {
            Err(RecipeDecision::Failed {
                stop_reason,
                failed_step,
            }) => Some((failed_step, stop_reason)),
            _ => None,
        }
    }

    fn selector_call(
        &self,
        params: &PlaylistParams,
        previous: Option<&ModelMessage>,
        errors: Option<Vec<String>>,
    ) -> RecipeDecision {
        let attempts = previous.map(|_| 1).unwrap_or(0);
        let session_id = previous.and_then(|result| {
            result
                .text
                .split_once("sessionID=\"")?
                .1
                .split_once('\"')
                .map(|(sid, _)| sid)
        });
        let source_kind = if params.video_urls.is_empty() {
            "YouTube playlist"
        } else {
            "provided YouTube videos"
        };
        let payload = json!({
            "task": format!("Choose a Wiki source folder domain/topic for these {source_kind}"),
            "requested_domain": params.domain.as_deref().unwrap_or(AGENT_AUTO_DOMAIN),
            "available_domains": self.wiki_domains(),
            "output_schema": TopicSelection::schema(),
            "diagnosis": errors.map(|errors| TopicSelectionDiagnosis {
                code: "INVALID_AGENT_STEP_OUTPUT",
                errors,
                expected: "JSON object with exactly string fields domain and topic; topic is a lowercase kebab-case slug",
            }),
        });
        let mut arguments = json!({
            "agent": PLAYLIST_DOMAIN_SELECTOR,
            "description": "Choose a Wiki domain and topic for the collected YouTube source items",
            "prompt": format!(
                "Use the only available completion tool `{PLAYLIST_COMPLETION_TOOL}` with the output fields `domain` and `topic`. Then return exactly the same canonical JSON object as your final response, with no prose or markdown. The delegated task is: {}",
                serde_json::to_string(&payload).unwrap_or_default()
            ),
        });
        if let Some(session_id) = session_id {
            arguments["session_id"] = Value::String(session_id.to_owned());
        }
        RecipeDecision::ToolCalls(vec![ModelToolCall {
            id: format!(
                "recipe-select-topic-{attempts}-{}",
                previous.map(|_| "resume").unwrap_or("start")
            ),
            name: "subagent".into(),
            arguments,
        }])
    }
}

impl TopicSelection {
    fn schema() -> Value {
        json!({
            "type": "object",
            "properties": {
                "domain": {"type": "string", "minLength": 1},
                "topic": {"type": "string", "pattern": "^[a-z0-9]+(?:-[a-z0-9]+)*$"}
            },
            "required": ["domain", "topic"],
            "additionalProperties": false
        })
    }
}

pub struct TopicSelectionCompletionTool;

#[async_trait]
impl HostTool for TopicSelectionCompletionTool {
    fn descriptor(&self) -> ToolDescriptor {
        ToolDescriptor {
            name: PLAYLIST_COMPLETION_TOOL.into(),
            description:
                "Complete the playlist domain and topic selection using the declared schema.".into(),
            input_schema: TopicSelection::schema(),
            effect: orchestrator_agent_rs::ToolEffect::ReadOnly,
        }
    }

    async fn execute(&self, context: ToolContext, args: Value) -> Result<ToolOutcome, ToolError> {
        let selection: TopicSelection =
            serde_json::from_value(args).map_err(|error| ToolError {
                code: "INVALID_COMPLETION_OUTPUT".into(),
                message: error.to_string(),
            })?;
        let errors = validate_topic_selection(&selection);
        if !errors.is_empty() {
            return Err(ToolError {
                code: "INVALID_COMPLETION_OUTPUT".into(),
                message: errors.join("; "),
            });
        }
        let output = serde_json::to_string(&selection).map_err(|error| ToolError {
            code: "INVALID_COMPLETION_OUTPUT".into(),
            message: error.to_string(),
        })?;
        Ok(ToolOutcome {
            call_id: context.scope().tool_call_id.clone(),
            content: output,
            metadata: json!({"completion_schema": "playlist_topic_selection:v1"}),
        })
    }
}

impl TaskRecipe for PlaylistSourcesRecipe {
    fn metadata(&self) -> RecipeMetadata {
        RecipeMetadata {
            id: PLAYLIST_RECIPE_ID.into(),
            version: 1,
            title: "Thu thập video vào sources".into(),
            description: "Liệt kê playlist hoặc URL video, lấy transcript theo lô, chọn domain/topic Wiki rồi lưu vào sources.".into(),
            params_schema: self.params_schema(),
            outline: vec![
                RecipeStepOutline {
                    title: "Lấy danh sách video".into(),
                    kind: RecipeStepKind::Tool,
                    repeat: false,
                    host_tools: vec!["yt_list_playlist".into(), "collection_status".into()],
                    sub_agents: vec![],
                },
                RecipeStepOutline {
                    title: "Lấy transcript theo lô".into(),
                    kind: RecipeStepKind::Tool,
                    repeat: true,
                    host_tools: vec!["yt_fetch_transcript".into()],
                    sub_agents: vec![],
                },
                RecipeStepOutline {
                    title: "Chọn domain và topic Wiki".into(),
                    kind: RecipeStepKind::Agent,
                    repeat: false,
                    host_tools: vec![],
                    sub_agents: vec![PLAYLIST_DOMAIN_SELECTOR.into()],
                },
                RecipeStepOutline {
                    title: "Lưu nguồn theo lô".into(),
                    kind: RecipeStepKind::Tool,
                    repeat: true,
                    host_tools: vec!["add_to_sources".into()],
                    sub_agents: vec![],
                },
            ],
            max_steps: 64,
        }
    }

    fn validate_params(&self, params: &Value) -> Result<Value, RecipeValidationError> {
        let params = validate_recipe_params(&self.params_schema(), params)?;
        let playlist_url = params
            .get("url")
            .and_then(Value::as_str)
            .filter(|url| !url.trim().is_empty());
        let video_urls = params
            .get("video_urls")
            .and_then(Value::as_array)
            .filter(|urls| !urls.is_empty());
        if playlist_url.is_some() == video_urls.is_some() {
            return Err(RecipeValidationError {
                validation_path: "params".into(),
                message: "provide exactly one of url or a non-empty video_urls list".into(),
            });
        }
        if let Some(url) = playlist_url
            && !Self::valid_youtube_url(url)
        {
            return Err(RecipeValidationError {
                validation_path: "params.url".into(),
                message: "value must be a YouTube playlist URL".into(),
            });
        }
        if let Some(urls) = video_urls {
            if urls.len() > MAX_VIDEOS {
                return Err(RecipeValidationError {
                    validation_path: "params.video_urls".into(),
                    message: format!("at most {MAX_VIDEOS} video URLs are allowed"),
                });
            }
            if let Some((index, _)) = urls
                .iter()
                .enumerate()
                .find(|(_, url)| !url.as_str().is_some_and(Self::valid_youtube_url))
            {
                return Err(RecipeValidationError {
                    validation_path: format!("params.video_urls.{index}"),
                    message: "value must be a YouTube video URL".into(),
                });
            }
        }
        Ok(params)
    }

    fn title_for_params(&self, params: &Value) -> Option<String> {
        Some(PlaylistParams::from_value(params).title())
    }

    fn next(&self, params: &Value, transcript: &[ModelMessage]) -> RecipeDecision {
        let params = &PlaylistParams::from_value(params);
        let urls = params.source_urls();
        if urls.is_empty() {
            return RecipeDecision::Failed {
                stop_reason: "playlist URL or video URL list is missing".into(),
                failed_step: 0,
            };
        }
        if let Some(error) = Self::failed_source_list(params, transcript) {
            return Self::decision_failure(error, 0);
        }
        let unlisted = urls
            .iter()
            .filter(|url| Self::list_result(transcript, url).is_none())
            .collect::<Vec<_>>();
        if !unlisted.is_empty() {
            return RecipeDecision::ToolCalls(
                unlisted
                    .into_iter()
                    .enumerate()
                    .map(|(index, url)| ModelToolCall {
                        id: format!("recipe-list-{index}-{}", transcript.len()),
                        name: "yt_list_playlist".into(),
                        arguments: json!({"url": url}),
                    })
                    .collect(),
            );
        }

        let statuses = urls
            .iter()
            .filter_map(|url| Self::status_for(transcript, url))
            .collect::<Vec<_>>();
        let status_index = Self::latest_result_index(transcript, &["collection_status"]);
        let latest_item_action =
            Self::latest_result_index(transcript, &["yt_fetch_transcript", "add_to_sources"]);
        if statuses.len() != urls.len()
            || latest_item_action.is_some_and(|index| status_index.is_none_or(|s| index > s))
        {
            return RecipeDecision::ToolCalls(
                urls.iter()
                    .enumerate()
                    .map(|(index, url)| ModelToolCall {
                        id: format!("recipe-collection-status-{index}-{}", transcript.len()),
                        name: "collection_status".into(),
                        arguments: json!({"playlist_url": url, "limit": BATCH_SIZE}),
                    })
                    .collect(),
            );
        }
        if let Some(status_error) = statuses
            .iter()
            .find(|status| status.text.trim_start().starts_with("error "))
        {
            return Self::decision_failure(status_error, 0);
        }

        let next = Self::status_items(params, transcript);
        let fetched = Self::successful_ids(transcript, "yt_fetch_transcript");
        let saved = Self::successful_ids(transcript, "add_to_sources");
        let pending_fetch = next
            .iter()
            .filter(|(id, _)| !fetched.contains(id))
            .take(BATCH_SIZE)
            .cloned()
            .collect::<Vec<_>>();
        if !pending_fetch.is_empty() {
            return RecipeDecision::ToolCalls(
                pending_fetch
                    .into_iter()
                    .map(|(video_id, title)| ModelToolCall {
                        id: format!("recipe-fetch-{video_id}-{}", transcript.len()),
                        name: "yt_fetch_transcript".into(),
                        arguments: json!({"video_id":video_id, "title":title}),
                    })
                    .collect(),
            );
        }

        let selection = match self.selector_decision(params, transcript) {
            Ok(selection) => selection,
            Err(decision) => return decision,
        };
        let pending_save = next
            .into_iter()
            .filter(|(id, _)| fetched.contains(id) && !saved.contains(id))
            .take(BATCH_SIZE)
            .collect::<Vec<_>>();
        if !pending_save.is_empty() {
            return RecipeDecision::ToolCalls(
                pending_save
                    .into_iter()
                    .map(|(video_id, title)| ModelToolCall {
                        id: format!("recipe-save-{video_id}"),
                        name: "add_to_sources".into(),
                        arguments: json!({
                            "topic_folder": format!("{}/{}", selection.domain, selection.topic),
                            "video_id": video_id,
                            "title": title,
                            "provenance": "source",
                            "ingest_to_wiki": params.ingest_to_wiki
                        }),
                    })
                    .collect(),
            );
        }

        RecipeDecision::Complete(format!("Đã lưu {} video vào sources.", saved.len()))
    }

    fn progress(&self, params: &Value, transcript: &[ModelMessage]) -> RecipeProgress {
        let params = &PlaylistParams::from_value(params);
        let urls = params.source_urls();
        let listed = !urls.is_empty()
            && urls.iter().all(|url| {
                Self::list_result(transcript, url)
                    .is_some_and(|result| !result.text.trim_start().starts_with("error "))
            });
        let statuses = urls
            .iter()
            .filter_map(|url| Self::status_for(transcript, url))
            .collect::<Vec<_>>();
        let fetched = Self::successful_ids(transcript, "yt_fetch_transcript");
        let saved = Self::successful_ids(transcript, "add_to_sources");
        let selected = Self::selector_results(transcript)
            .last()
            .is_some_and(|(_, result)| Self::selector_output(result).is_ok());
        let failed_list = Self::failed_source_list(params, transcript);
        let failed_status = statuses
            .iter()
            .copied()
            .find(|result| result.text.trim_start().starts_with("error "));
        let failure = failed_list
            .or(failed_status)
            .map(|result| {
                (
                    0,
                    result
                        .text
                        .lines()
                        .next()
                        .unwrap_or("recipe tool failed")
                        .to_owned(),
                )
            })
            .or_else(|| self.selector_failure(params, transcript));
        if statuses.is_empty() && failure.is_none() {
            return RecipeProgress {
                current_step: Some(0),
                completed_steps: vec![],
                completed_items: None,
                total_items: None,
                summary: None,
                stop_reason: None,
                failed_step: None,
            };
        }
        let items = Self::status_items(params, transcript);
        let current_step = if failure.is_some() {
            None
        } else if !listed || statuses.len() != urls.len() {
            Some(0)
        } else if items.iter().any(|(id, _)| !fetched.contains(id)) {
            Some(1)
        } else if !selected {
            Some(2)
        } else if items
            .iter()
            .any(|(id, _)| fetched.contains(id) && !saved.contains(id))
        {
            Some(3)
        } else {
            None
        };
        let total_items = statuses
            .iter()
            .filter_map(|status| {
                status
                    .text
                    .split_once("TOTAL:")
                    .and_then(|(_, rest)| {
                        rest.trim_start()
                            .split(|ch: char| !ch.is_ascii_digit())
                            .next()
                    })
                    .and_then(|count| count.parse::<usize>().ok())
            })
            .sum::<usize>();
        let (completed_items, current_total) = match current_step {
            Some(1) => (Some(fetched.len()), total_items),
            Some(2 | 3) => (Some(saved.len()), total_items),
            _ => (Some(total_items), total_items),
        };
        let completed_steps = if listed { vec![0] } else { vec![] };
        let completed_steps = if current_step.is_none() && failure.is_none() {
            vec![0, 1, 2, 3]
        } else if current_step == Some(3) {
            vec![0, 1, 2]
        } else if current_step == Some(2) {
            vec![0, 1]
        } else {
            completed_steps
        };
        RecipeProgress {
            current_step,
            completed_steps,
            completed_items,
            total_items: Some(current_total),
            summary: Some(format!(
                "Transcript {} · đã lưu {}",
                fetched.len(),
                saved.len()
            )),
            stop_reason: failure.as_ref().map(|(_, reason)| reason.clone()),
            failed_step: failure.map(|(step, _)| step),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use orchestrator_agent_rs::{ModelMessage, ModelToolCall};
    use serde_json::json;

    fn ready_transcript() -> Vec<ModelMessage> {
        vec![
            ModelMessage::tool_result("list", "yt_list_playlist", "listed"),
            ModelMessage::tool_result(
                "status-1",
                "collection_status",
                "TOTAL: 1 | REMAINING: 1\nNEXT:\n1. video_id=v1 title=One",
            ),
            ModelMessage::assistant_with_calls(
                "",
                vec![ModelToolCall {
                    id: "fetch-v1".into(),
                    name: "yt_fetch_transcript".into(),
                    arguments: json!({"video_id":"v1"}),
                }],
            ),
            ModelMessage::tool_result("fetch-v1", "yt_fetch_transcript", "TRANSCRIPT"),
            ModelMessage::tool_result(
                "status-2",
                "collection_status",
                "TOTAL: 1 | REMAINING: 1\nNEXT:\n1. video_id=v1 title=One",
            ),
        ]
    }

    fn selector_result(call_id: &str, session_id: &str, output: &str) -> Vec<ModelMessage> {
        vec![
            ModelMessage::assistant_with_calls(
                "",
                vec![ModelToolCall {
                    id: call_id.into(),
                    name: "subagent".into(),
                    arguments: json!({"agent":PLAYLIST_DOMAIN_SELECTOR}),
                }],
            ),
            ModelMessage::tool_result(
                call_id,
                "subagent",
                format!(
                    "<subagent sessionID=\"{session_id}\" state=\"completed\">\n{output}\n</subagent>"
                ),
            ),
        ]
    }

    #[path = "ledger.rs"]
    mod ledger;
    #[path = "progress.rs"]
    mod progress;
    #[path = "selection.rs"]
    mod selection;
    #[path = "validation.rs"]
    mod validation;
}
