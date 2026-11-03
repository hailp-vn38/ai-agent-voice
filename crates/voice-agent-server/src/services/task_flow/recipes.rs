use std::collections::HashSet;

use orchestrator_agent_rs::{ModelMessage, ModelRole, ModelToolCall};
use orchestrator_flow_rs::{
    RecipeDecision, RecipeMetadata, RecipeProgress, RecipeStepKind, RecipeStepOutline,
    RecipeValidationError, TaskRecipe, validate_recipe_params,
};
use serde_json::{Value, json};

pub const PLAYLIST_RECIPE_ID: &str = "playlist_sources";
pub const PLAYLIST_RECIPE_ROOT: &str = "recipe_playlist_sources";
const BATCH_SIZE: usize = 10;
const MAX_VIDEOS: usize = 50;

/// First server-owned recipe. It lists once, fetches bounded transcript batches, then saves
/// successful items through the ordinary mutating HostTool pipeline.
pub struct PlaylistSourcesRecipe;

impl PlaylistSourcesRecipe {
    fn params_schema() -> Value {
        json!({
            "type": "object",
            "properties": {
                "url": {"type": "string", "format": "uri", "title": "Playlist URL"},
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
}

impl TaskRecipe for PlaylistSourcesRecipe {
    fn metadata(&self) -> RecipeMetadata {
        RecipeMetadata {
            id: PLAYLIST_RECIPE_ID.into(),
            version: 1,
            title: "Thu thập playlist vào sources".into(),
            description:
                "Liệt kê video, lấy transcript theo lô và lưu vào sources/youtube/playlist.".into(),
            params_schema: Self::params_schema(),
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
        validate_recipe_params(&Self::params_schema(), params)
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
        let pending_save = next
            .iter()
            .filter(|(id, _)| fetched.contains(id) && !saved.contains(id))
            .take(BATCH_SIZE)
            .cloned()
            .collect::<Vec<_>>();
        if !pending_save.is_empty() {
            return RecipeDecision::ToolCalls(pending_save.into_iter().map(|(video_id, title)| ModelToolCall {
                id: format!("recipe-save-{video_id}"),
                name: "add_to_sources".into(),
                arguments: json!({"topic_folder":"youtube/playlist", "video_id":video_id, "title":title, "provenance":"source"}),
            }).collect());
        }

        let pending_fetch = next
            .into_iter()
            .filter(|(id, _)| !fetched.contains(id))
            .take(BATCH_SIZE)
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

        RecipeDecision::Complete(format!("Đã lưu {} video vào sources.", saved.len()))
    }

    fn progress(&self, params: &Value, transcript: &[ModelMessage]) -> RecipeProgress {
        let listed = Self::latest_list(transcript)
            .is_some_and(|result| !result.text.trim_start().starts_with("error "));
        let Some((_, status)) = Self::latest_status(transcript) else {
            return RecipeProgress {
                current_step: Some(0),
                completed_steps: vec![],
                summary: None,
                stop_reason: None,
                failed_step: None,
            };
        };
        let items = Self::next_items(&status.text, Self::max_videos(params));
        let fetched = Self::successful_ids(transcript, "yt_fetch_transcript");
        let saved = Self::successful_ids(transcript, "add_to_sources");
        let current_step = if !listed {
            Some(0)
        } else if items
            .iter()
            .any(|(id, _)| fetched.contains(id) && !saved.contains(id))
        {
            Some(2)
        } else if !items.is_empty() {
            Some(1)
        } else {
            None
        };
        RecipeProgress {
            current_step,
            completed_steps: if listed { vec![0] } else { vec![] },
            summary: Some(format!(
                "Transcript {} · đã lưu {}",
                fetched.len(),
                saved.len()
            )),
            stop_reason: None,
            failed_step: None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn outline_separates_list_fetch_and_save_steps() {
        let outline = PlaylistSourcesRecipe.metadata().outline;
        assert_eq!(outline.len(), 3);
        assert_eq!(
            outline[0].host_tools,
            ["yt_list_playlist", "collection_status"]
        );
        assert_eq!(outline[1].host_tools, ["yt_fetch_transcript"]);
        assert!(outline[1].repeat);
        assert_eq!(outline[2].host_tools, ["add_to_sources"]);
        assert!(outline[2].repeat);
    }

    #[test]
    fn recipe_fetches_then_saves_a_bounded_ledger_batch() {
        let recipe = PlaylistSourcesRecipe;
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
            panic!("expected source save")
        };
        assert_eq!(calls[0].name, "add_to_sources");
    }

    #[test]
    fn failed_video_does_not_prevent_successful_sibling_from_being_saved() {
        let recipe = PlaylistSourcesRecipe;
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
            panic!("good sibling should be saved")
        };
        assert_eq!(calls.len(), 1);
        assert_eq!(calls[0].name, "add_to_sources");
        assert_eq!(calls[0].arguments["video_id"], "good");
    }
}
