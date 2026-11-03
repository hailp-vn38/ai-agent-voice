use std::collections::HashSet;

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
const AGENT_AUTO_DOMAIN: &str = "agent tự chọn";

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

/// First server-owned recipe. It lists once, fetches bounded transcript batches, then saves
/// successful items through the ordinary mutating HostTool pipeline.
#[derive(Default)]
pub struct PlaylistSourcesRecipe {
    wiki_domains: Vec<String>,
}

impl PlaylistSourcesRecipe {
    pub fn new(wiki_domains: Vec<String>) -> Self {
        let mut wiki_domains = wiki_domains
            .into_iter()
            .map(|domain| domain.trim().to_owned())
            .filter(|domain| !domain.is_empty() && domain != AGENT_AUTO_DOMAIN)
            .collect::<Vec<_>>();
        wiki_domains.sort();
        wiki_domains.dedup();
        Self { wiki_domains }
    }

    fn params_schema(&self) -> Value {
        let mut domains = vec![Value::String(AGENT_AUTO_DOMAIN.into())];
        domains.extend(self.wiki_domains.iter().cloned().map(Value::String));
        json!({
            "type": "object",
            "properties": {
                "url": {"type": "string", "format": "uri", "title": "Playlist URL"},
                "domain": {
                    "type": "string",
                    "title": "Wiki domain",
                    "default": AGENT_AUTO_DOMAIN,
                    "enum": domains,
                    "x-options-source": "wiki_domains"
                },
                "max_videos": {"type": "integer", "minimum": 1, "maximum": 50, "default": 50, "title": "Số video tối đa"}
            },
            "required": ["url"]
        })
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

    fn max_videos(params: &Value) -> usize {
        params
            .get("max_videos")
            .and_then(Value::as_u64)
            .unwrap_or(MAX_VIDEOS as u64)
            .min(MAX_VIDEOS as u64) as usize
    }

    fn failed_list(transcript: &[ModelMessage]) -> Option<&ModelMessage> {
        Self::latest_list(transcript)
            .filter(|result| result.text.trim_start().starts_with("error "))
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

    fn selector_results<'a>(
        transcript: &'a [ModelMessage],
    ) -> Vec<(&'a ModelToolCall, &'a ModelMessage)> {
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
        if selection.domain.trim().is_empty() {
            return Err(vec!["domain must be a non-empty Wiki domain".into()]);
        }
        if selection.topic.is_empty()
            || !selection
                .topic
                .bytes()
                .all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit() || byte == b'-')
            || selection.topic.starts_with('-')
            || selection.topic.ends_with('-')
        {
            return Err(vec!["topic must be a lowercase kebab-case slug".into()]);
        }
        Ok(selection)
    }

    fn selector_decision(
        &self,
        params: &Value,
        transcript: &[ModelMessage],
    ) -> Result<TopicSelection, RecipeDecision> {
        let attempts = Self::selector_results(transcript);
        if let Some((_, result)) = attempts.last() {
            if !result.text.trim_start().starts_with("error ") {
                match Self::selector_output(result) {
                    Ok(selection) => {
                        let requested_domain = params
                            .get("domain")
                            .and_then(Value::as_str)
                            .unwrap_or(AGENT_AUTO_DOMAIN);
                        if requested_domain != AGENT_AUTO_DOMAIN
                            && selection.domain != requested_domain
                        {
                            return Err(RecipeDecision::Failed {
                                stop_reason: "RECIPE_STEP_FAILED: agent selected a different domain than requested".into(),
                                failed_step: 2,
                            });
                        }
                        if requested_domain == AGENT_AUTO_DOMAIN
                            && !self.wiki_domains.is_empty()
                            && !self.wiki_domains.contains(&selection.domain)
                        {
                            return Err(RecipeDecision::Failed {
                                stop_reason: "RECIPE_STEP_FAILED: agent selected a domain outside the current Wiki catalog".into(),
                                failed_step: 2,
                            });
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
        params: &Value,
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
        params: &Value,
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
        let domain = params
            .get("domain")
            .and_then(Value::as_str)
            .unwrap_or(AGENT_AUTO_DOMAIN);
        let payload = json!({
            "task": "Choose a Wiki source folder domain/topic for this playlist",
            "requested_domain": domain,
            "available_domains": self.wiki_domains,
            "output_schema": TopicSelection::schema(),
            "diagnosis": errors.map(|errors| TopicSelectionDiagnosis {
                code: "INVALID_AGENT_STEP_OUTPUT",
                errors,
                expected: "JSON object with exactly string fields domain and topic; topic is a lowercase kebab-case slug",
            }),
        });
        let mut arguments = json!({
            "agent": PLAYLIST_DOMAIN_SELECTOR,
            "description": "Choose a Wiki domain and topic for the playlist",
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
        if selection.domain.trim().is_empty()
            || selection.topic.is_empty()
            || !selection
                .topic
                .bytes()
                .all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit() || byte == b'-')
            || selection.topic.starts_with('-')
            || selection.topic.ends_with('-')
        {
            return Err(ToolError {
                code: "INVALID_COMPLETION_OUTPUT".into(),
                message: "domain must be non-empty and topic must be a lowercase kebab-case slug"
                    .into(),
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
            title: "Thu thập playlist vào sources".into(),
            description:
                "Liệt kê video, lấy transcript theo lô, chọn domain/topic Wiki rồi lưu vào sources."
                    .into(),
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
        validate_recipe_params(&self.params_schema(), params)
    }

    fn next(&self, params: &Value, transcript: &[ModelMessage]) -> RecipeDecision {
        let Some(url) = params.get("url").and_then(Value::as_str) else {
            return RecipeDecision::Failed {
                stop_reason: "playlist URL is missing".into(),
                failed_step: 0,
            };
        };
        if let Some(error) = Self::failed_list(transcript) {
            return Self::decision_failure(error, 0);
        }
        if Self::latest_list(transcript).is_none() {
            return RecipeDecision::ToolCalls(vec![ModelToolCall {
                id: "recipe-list-playlist".into(),
                name: "yt_list_playlist".into(),
                arguments: json!({"url": url}),
            }]);
        }

        let Some((status_index, status)) = Self::latest_status(transcript) else {
            return RecipeDecision::ToolCalls(vec![ModelToolCall {
                id: "recipe-collection-status-initial".into(),
                name: "collection_status".into(),
                arguments: json!({"playlist_url": url, "limit": BATCH_SIZE}),
            }]);
        };
        let latest_item_action =
            Self::latest_result_index(transcript, &["yt_fetch_transcript", "add_to_sources"]);
        if latest_item_action.is_some_and(|index| index > status_index) {
            return RecipeDecision::ToolCalls(vec![ModelToolCall {
                id: format!("recipe-collection-status-{}", transcript.len()),
                name: "collection_status".into(),
                arguments: json!({"playlist_url": url, "limit": BATCH_SIZE}),
            }]);
        }
        if status.text.trim_start().starts_with("error ") {
            return Self::decision_failure(status, 0);
        }

        let next = Self::next_items(&status.text, Self::max_videos(params));
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
                            "provenance": "source"
                        }),
                    })
                    .collect(),
            );
        }

        RecipeDecision::Complete(format!("Đã lưu {} video vào sources.", saved.len()))
    }

    fn progress(&self, params: &Value, transcript: &[ModelMessage]) -> RecipeProgress {
        let listed = Self::latest_list(transcript)
            .is_some_and(|result| !result.text.trim_start().starts_with("error "));
        let Some((_, status)) = Self::latest_status(transcript) else {
            if let Some(failed) = Self::failed_list(transcript) {
                return RecipeProgress {
                    current_step: None,
                    completed_steps: vec![],
                    completed_items: None,
                    total_items: None,
                    summary: None,
                    stop_reason: Some(
                        failed
                            .text
                            .lines()
                            .next()
                            .unwrap_or("recipe tool failed")
                            .to_owned(),
                    ),
                    failed_step: Some(0),
                };
            }
            return RecipeProgress {
                current_step: Some(0),
                completed_steps: vec![],
                completed_items: None,
                total_items: None,
                summary: None,
                stop_reason: None,
                failed_step: None,
            };
        };
        let items = Self::next_items(&status.text, Self::max_videos(params));
        let fetched = Self::successful_ids(transcript, "yt_fetch_transcript");
        let saved = Self::successful_ids(transcript, "add_to_sources");
        let selected = Self::selector_results(transcript)
            .last()
            .is_some_and(|(_, result)| Self::selector_output(result).is_ok());
        let stopped = Self::failed_list(transcript).or_else(|| {
            Self::latest_status(transcript)
                .map(|(_, result)| result)
                .filter(|result| result.text.trim_start().starts_with("error "))
        });
        let failure = stopped
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
        let current_step = if failure.is_some() {
            None
        } else if !listed {
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
        let total_items = status
            .text
            .split_once("TOTAL:")
            .and_then(|(_, rest)| {
                rest.trim_start()
                    .split(|ch: char| !ch.is_ascii_digit())
                    .next()
            })
            .and_then(|count| count.parse::<usize>().ok());
        let (completed_items, current_total) = match current_step {
            Some(1) => (Some(fetched.len()), total_items),
            Some(2) => (Some(saved.len()), total_items),
            _ => (total_items, total_items),
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
            total_items: current_total,
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

    #[test]
    fn outline_separates_list_fetch_and_save_steps() {
        let outline = PlaylistSourcesRecipe::default().metadata().outline;
        assert_eq!(outline.len(), 4);
        assert_eq!(
            outline[0].host_tools,
            ["yt_list_playlist", "collection_status"]
        );
        assert_eq!(outline[1].host_tools, ["yt_fetch_transcript"]);
        assert!(outline[1].repeat);
        assert_eq!(outline[2].kind, RecipeStepKind::Agent);
        assert_eq!(outline[2].sub_agents, [PLAYLIST_DOMAIN_SELECTOR]);
        assert_eq!(outline[3].host_tools, ["add_to_sources"]);
        assert!(outline[3].repeat);
    }

    #[test]
    fn playlist_agent_step_starts_then_accepts_only_typed_json_completion() {
        let recipe = PlaylistSourcesRecipe::new(vec!["technology".into()]);
        let params = json!({"url":"https://www.youtube.com/playlist?list=PL1"});
        let trailing_prose = ModelMessage::tool_result(
            "bad-envelope",
            "subagent",
            "<subagent sessionID=\"s\" state=\"completed\">\n{\"domain\":\"technology\",\"topic\":\"ml\"}\n</subagent> extra",
        );
        assert!(PlaylistSourcesRecipe::selector_output(&trailing_prose).is_err());
        let mut transcript = ready_transcript();

        let RecipeDecision::ToolCalls(calls) = recipe.next(&params, &transcript) else {
            panic!("expected the selector subagent")
        };
        assert_eq!(calls[0].name, "subagent");
        assert_eq!(calls[0].arguments["agent"], PLAYLIST_DOMAIN_SELECTOR);
        assert!(
            calls[0].arguments["prompt"]
                .as_str()
                .unwrap()
                .contains("technology")
        );

        transcript.extend(selector_result(
            &calls[0].id,
            "selector-session",
            r#"{"domain":"technology","topic":"machine-learning"}"#,
        ));
        let RecipeDecision::ToolCalls(save_calls) = recipe.next(&params, &transcript) else {
            panic!("expected source save after typed completion")
        };
        assert_eq!(save_calls[0].name, "add_to_sources");
        assert_eq!(
            save_calls[0].arguments["topic_folder"],
            "technology/machine-learning"
        );
    }

    #[test]
    fn invalid_agent_output_resumes_once_with_typed_diagnosis_then_stops() {
        let recipe = PlaylistSourcesRecipe::new(vec!["technology".into()]);
        let params = json!({"url":"https://www.youtube.com/playlist?list=PL1"});
        let mut transcript = ready_transcript();
        let RecipeDecision::ToolCalls(first) = recipe.next(&params, &transcript) else {
            panic!("expected first selector call")
        };
        transcript.extend(selector_result(
            &first[0].id,
            "selector-session",
            r#"{"domain":false,"topic":"Bad Topic"}"#,
        ));

        let RecipeDecision::ToolCalls(resume) = recipe.next(&params, &transcript) else {
            panic!("expected one corrective resume")
        };
        assert_eq!(resume[0].name, "subagent");
        assert_eq!(resume[0].arguments["session_id"], "selector-session");
        let correction = resume[0].arguments["prompt"].as_str().unwrap();
        assert!(correction.contains("INVALID_AGENT_STEP_OUTPUT"));
        assert!(
            correction.contains("domain must be a non-empty Wiki domain")
                || correction.contains("completion output must match")
        );

        transcript.extend(selector_result(
            &resume[0].id,
            "selector-session",
            r#"{"domain":"technology","topic":"Bad Topic"}"#,
        ));
        let RecipeDecision::Failed {
            stop_reason,
            failed_step,
        } = recipe.next(&params, &transcript)
        else {
            panic!("the second invalid output must stop the recipe")
        };
        assert_eq!(failed_step, 2);
        assert!(stop_reason.starts_with("RECIPE_STEP_FAILED:"));
        assert!(stop_reason.contains("INVALID_AGENT_STEP_OUTPUT"));
        let progress = recipe.progress(&params, &transcript);
        assert_eq!(progress.current_step, None);
        assert_eq!(progress.failed_step, Some(2));
        assert!(
            progress
                .stop_reason
                .unwrap()
                .contains("INVALID_AGENT_STEP_OUTPUT")
        );
    }

    #[test]
    fn playlist_domain_options_are_dynamic_and_annotated_for_the_admin_form() {
        let recipe = PlaylistSourcesRecipe::new(vec!["science".into(), "technology".into()]);
        let metadata = recipe.metadata();
        let domain = &metadata.params_schema["properties"]["domain"];
        assert_eq!(domain["x-options-source"], "wiki_domains");
        assert_eq!(domain["enum"][0], AGENT_AUTO_DOMAIN);
        assert_eq!(domain["enum"][1], "science");
        assert_eq!(domain["enum"][2], "technology");
        assert_eq!(
            recipe
                .validate_params(&json!({
                    "url":"https://www.youtube.com/playlist?list=PL1",
                    "domain":"technology"
                }))
                .unwrap()["domain"],
            "technology"
        );
    }

    #[test]
    fn playlist_progress_reports_repeated_items_and_completion_from_transcript() {
        let recipe = PlaylistSourcesRecipe::default();
        let params = json!({"url":"https://www.youtube.com/playlist?list=PL1"});
        let list = ModelMessage::tool_result("list", "yt_list_playlist", "listed");
        let status = ModelMessage::tool_result(
            "status",
            "collection_status",
            "TOTAL: 2 | REMAINING: 2\nNEXT:\n1. video_id=v1 title=One",
        );
        let fetched = ModelMessage::tool_result("fetch-v1", "yt_fetch_transcript", "saved v1");
        let running = vec![
            ModelMessage::assistant_with_calls(
                "",
                vec![ModelToolCall {
                    id: "fetch-v1".into(),
                    name: "yt_fetch_transcript".into(),
                    arguments: json!({"video_id":"v1"}),
                }],
            ),
            list.clone(),
            status,
            fetched,
        ];

        let progress = recipe.progress(&params, &running);
        assert_eq!(progress.current_step, Some(2));
        assert_eq!(progress.completed_items, Some(0));
        assert_eq!(progress.total_items, Some(2));
        assert!(progress.stop_reason.is_none());

        let completed_status = ModelMessage::tool_result(
            "status-done",
            "collection_status",
            "TOTAL: 1 | REMAINING: 0\nNEXT:",
        );
        let selection = selector_result(
            "select-topic",
            "selector-session",
            r#"{"domain":"youtube","topic":"playlist"}"#,
        );
        let completed = vec![
            list,
            completed_status,
            selection[0].clone(),
            selection[1].clone(),
            ModelMessage::assistant_with_calls(
                "",
                vec![
                    ModelToolCall {
                        id: "fetch-one".into(),
                        name: "yt_fetch_transcript".into(),
                        arguments: json!({"video_id":"v1"}),
                    },
                    ModelToolCall {
                        id: "save-one".into(),
                        name: "add_to_sources".into(),
                        arguments: json!({"video_id":"v1"}),
                    },
                ],
            ),
            ModelMessage::tool_result("fetch-one", "yt_fetch_transcript", "ok"),
            ModelMessage::tool_result("save-one", "add_to_sources", "ok"),
        ];
        let progress = recipe.progress(&params, &completed);
        assert_eq!(progress.current_step, None);
        assert_eq!(progress.completed_steps, vec![0, 1, 2, 3]);
        assert_eq!(progress.completed_items, Some(1));
        assert_eq!(progress.total_items, Some(1));
        assert!(progress.stop_reason.is_none());
    }

    #[test]
    fn playlist_progress_reports_stopped_step_and_reason() {
        let recipe = PlaylistSourcesRecipe::default();
        let params = json!({"url":"https://www.youtube.com/playlist?list=PL1"});
        let transcript = vec![ModelMessage::tool_result(
            "list",
            "yt_list_playlist",
            "error playlist is private",
        )];

        let progress = recipe.progress(&params, &transcript);
        assert_eq!(progress.current_step, None);
        assert_eq!(
            progress.stop_reason.as_deref(),
            Some("error playlist is private")
        );
        assert_eq!(progress.failed_step, Some(0));
    }

    #[test]
    fn recipe_fetches_then_saves_a_bounded_ledger_batch() {
        let recipe = PlaylistSourcesRecipe::default();
        let params = json!({"url":"https://www.youtube.com/playlist?list=PL1"});
        let listed =
            ModelMessage::tool_result("list", "yt_list_playlist", "1. video_id=v1 title=One");
        let decision = recipe.next(&params, &[listed.clone()]);
        let RecipeDecision::ToolCalls(calls) = decision else {
            panic!("expected status lookup")
        };
        assert_eq!(calls[0].name, "collection_status");
        let status = ModelMessage::tool_result(
            "status",
            "collection_status",
            "TOTAL: 1 | REMAINING: 1\nNEXT:\n1. video_id=v1 attempts=0 title=One",
        );
        let decision = recipe.next(&params, &[listed.clone(), status.clone()]);
        let RecipeDecision::ToolCalls(calls) = decision else {
            panic!("expected transcript fetch")
        };
        assert_eq!(calls[0].name, "yt_fetch_transcript");

        let fetch_call = ModelMessage::assistant_with_calls("", vec![calls[0].clone()]);
        let fetched =
            ModelMessage::tool_result(calls[0].id.clone(), "yt_fetch_transcript", "TRANSCRIPT");
        let transcript = vec![listed, status, fetch_call, fetched];
        let decision = recipe.next(&params, &transcript);
        let RecipeDecision::ToolCalls(calls) = decision else {
            panic!("expected refreshed collection status")
        };
        assert_eq!(calls[0].name, "collection_status");

        let refresh_call = ModelMessage::assistant_with_calls("", vec![calls[0].clone()]);
        let refreshed = ModelMessage::tool_result(
            calls[0].id.clone(),
            "collection_status",
            "TOTAL: 1 | REMAINING: 1\nNEXT:\n1. video_id=v1 attempts=0 title=One",
        );
        let mut transcript = [transcript, vec![refresh_call, refreshed]].concat();
        transcript.extend(selector_result(
            "select-topic",
            "selector-session",
            r#"{"domain":"youtube","topic":"playlist"}"#,
        ));
        let decision = recipe.next(&params, &transcript);
        let RecipeDecision::ToolCalls(calls) = decision else {
            panic!("expected source save after refresh")
        };
        assert_eq!(calls[0].name, "add_to_sources");
    }

    #[test]
    fn failed_video_does_not_prevent_successful_sibling_from_being_saved() {
        let recipe = PlaylistSourcesRecipe::default();
        let params = json!({"url":"https://www.youtube.com/playlist?list=PL1"});
        let list_call = ModelToolCall {
            id: "list".into(),
            name: "yt_list_playlist".into(),
            arguments: json!({"url":"https://www.youtube.com/playlist?list=PL1"}),
        };
        let list = ModelMessage::assistant_with_calls("", vec![list_call]);
        let listed = ModelMessage::tool_result("list", "yt_list_playlist", "listed");
        let status_call = ModelToolCall {
            id: "status".into(),
            name: "collection_status".into(),
            arguments: json!({"playlist_url":"https://www.youtube.com/playlist?list=PL1"}),
        };
        let status_assistant = ModelMessage::assistant_with_calls("", vec![status_call]);
        let status = ModelMessage::tool_result(
            "status",
            "collection_status",
            "TOTAL: 2 | REMAINING: 2\nNEXT:\n1. video_id=bad attempts=0 title=Bad\n2. video_id=good attempts=0 title=Good",
        );
        let fetch_calls = vec![
            ModelToolCall {
                id: "f-bad".into(),
                name: "yt_fetch_transcript".into(),
                arguments: json!({"video_id":"bad", "title":"Bad"}),
            },
            ModelToolCall {
                id: "f-good".into(),
                name: "yt_fetch_transcript".into(),
                arguments: json!({"video_id":"good", "title":"Good"}),
            },
        ];
        let fetch_assistant = ModelMessage::assistant_with_calls("", fetch_calls);
        let failed = ModelMessage::tool_result(
            "f-bad",
            "yt_fetch_transcript",
            "error NO_TRANSCRIPT: unavailable",
        );
        let success = ModelMessage::tool_result("f-good", "yt_fetch_transcript", "TRANSCRIPT");
        let transcript = vec![
            list,
            listed,
            status_assistant,
            status,
            fetch_assistant,
            failed,
            success,
        ];
        let decision = recipe.next(&params, &transcript);
        let RecipeDecision::ToolCalls(calls) = decision else {
            panic!("expected refreshed collection status")
        };
        assert_eq!(calls.len(), 1);
        assert_eq!(calls[0].name, "collection_status");

        let refresh_call = ModelMessage::assistant_with_calls("", vec![calls[0].clone()]);
        let refreshed = ModelMessage::tool_result(
            calls[0].id.clone(),
            "collection_status",
            "TOTAL: 2 | REMAINING: 1\nNEXT:\n2. video_id=good attempts=0 title=Good",
        );
        let mut transcript = [transcript, vec![refresh_call, refreshed]].concat();
        transcript.extend(selector_result(
            "select-topic",
            "selector-session",
            r#"{"domain":"youtube","topic":"playlist"}"#,
        ));
        let decision = recipe.next(&params, &transcript);
        let RecipeDecision::ToolCalls(calls) = decision else {
            panic!("good sibling should be saved after refresh")
        };
        assert_eq!(calls.len(), 1);
        assert_eq!(calls[0].name, "add_to_sources");
        assert_eq!(calls[0].arguments["video_id"], "good");
    }

    #[test]
    fn compacted_transcript_uses_the_durable_ledger_for_the_next_batch() {
        let recipe = PlaylistSourcesRecipe::default();
        let params = json!({
            "url":"https://www.youtube.com/playlist?list=PL1",
            "max_videos":50
        });
        let compacted = ModelMessage::text(
            ModelRole::Assistant,
            "Earlier batches were saved; continue from the durable collection ledger.",
        );
        let listed = ModelMessage::tool_result(
            "list",
            "yt_list_playlist",
            "Playlist listed; collection ledger is durable.",
        );
        let mut next_items = String::from("TOTAL: 50 | REMAINING: 30\nNEXT:\n");
        for position in 21..=50 {
            next_items.push_str(&format!(
                "{position}. video_id=v{position} attempts=0 title=Video {position}\n"
            ));
        }
        let ledger = ModelMessage::tool_result("status-next", "collection_status", next_items);

        let RecipeDecision::ToolCalls(calls) = recipe.next(&params, &[compacted, listed, ledger])
        else {
            panic!("the compacted transcript should select remaining ledger items")
        };
        assert_eq!(calls.len(), 10);
        assert!(calls.iter().all(|call| call.name == "yt_fetch_transcript"));
        assert_eq!(calls.first().unwrap().arguments["video_id"], "v21");
        assert_eq!(calls.last().unwrap().arguments["video_id"], "v30");
    }
}
