use orchestrator_agent_rs::{ModelMessage, ModelRole, ModelToolCall};
use orchestrator_flow_rs::{
    RecipeDecision, RecipeMetadata, RecipeProgress, RecipeStepKind, RecipeStepOutline,
    RecipeValidationError, TaskRecipe, validate_recipe_params,
};
use serde_json::{Value, json};

pub const PLAYLIST_RECIPE_ID: &str = "playlist_sources";
pub const PLAYLIST_RECIPE_ROOT: &str = "recipe_playlist_sources";

/// First server-owned recipe. The destination is intentionally fixed for this tracer.
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

    fn list_result(transcript: &[ModelMessage]) -> Option<&str> {
        transcript
            .iter()
            .rev()
            .find(|message| {
                message.role == ModelRole::Tool
                    && message.tool_name.as_deref() == Some("yt_list_playlist")
            })
            .map(|message| message.text.as_str())
    }

    fn listed_videos(text: &str) -> Vec<(String, String)> {
        text.lines()
            .filter_map(|line| {
                let id = line.split("video_id=").nth(1)?.split_whitespace().next()?;
                if line.contains("status=already_ingested") || line.contains("status=duplicate_of")
                {
                    return None;
                }
                let title = line.split("title=").nth(1).unwrap_or(id).trim().to_owned();
                Some((id.to_owned(), title))
            })
            .collect()
    }

    fn emitted_video_ids(transcript: &[ModelMessage]) -> Vec<String> {
        transcript
            .iter()
            .flat_map(|message| message.tool_calls.iter())
            .filter(|call| call.name == "add_to_sources")
            .filter_map(|call| {
                call.arguments
                    .get("video_id")
                    .and_then(Value::as_str)
                    .map(str::to_owned)
            })
            .collect()
    }

    fn completed_add_calls(transcript: &[ModelMessage]) -> usize {
        let completed: std::collections::HashSet<&str> = transcript
            .iter()
            .filter(|message| message.role == ModelRole::Tool)
            .filter_map(|message| message.tool_call_id.as_deref())
            .collect();
        transcript
            .iter()
            .flat_map(|message| message.tool_calls.iter())
            .filter(|call| call.name == "add_to_sources" && completed.contains(call.id.as_str()))
            .count()
    }

    fn failed_tool_result(transcript: &[ModelMessage]) -> Option<&ModelMessage> {
        transcript.iter().rev().find(|message| {
            message.role == ModelRole::Tool && message.text.trim_start().starts_with("error ")
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
                "Lấy danh sách video YouTube và lưu transcript vào sources/youtube/playlist.".into(),
            params_schema: Self::params_schema(),
            outline: vec![
                RecipeStepOutline {
                    title: "Lấy danh sách video".into(),
                    kind: RecipeStepKind::Tool,
                    repeat: false,
                },
                RecipeStepOutline {
                    title: "Lưu transcript vào sources".into(),
                    kind: RecipeStepKind::Tool,
                    repeat: true,
                },
            ],
            max_steps: 8,
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
        let Some(list_result) = Self::list_result(transcript) else {
            return RecipeDecision::ToolCalls(vec![ModelToolCall {
                id: "recipe-list-playlist".into(),
                name: "yt_list_playlist".into(),
                arguments: json!({"url": url}),
            }]);
        };

        if let Some(failed) = Self::failed_tool_result(transcript) {
            let failed_step = if failed.tool_name.as_deref() == Some("yt_list_playlist") {
                0
            } else {
                1
            };
            return RecipeDecision::Failed {
                stop_reason: failed
                    .text
                    .lines()
                    .next()
                    .unwrap_or("recipe tool failed")
                    .to_owned(),
                failed_step,
            };
        }

        let max_videos = params
            .get("max_videos")
            .and_then(Value::as_u64)
            .unwrap_or(50)
            .min(50) as usize;
        let videos = Self::listed_videos(list_result)
            .into_iter()
            .take(max_videos)
            .collect::<Vec<_>>();
        let emitted: std::collections::HashSet<String> =
            Self::emitted_video_ids(transcript).into_iter().collect();
        let calls = videos
            .into_iter()
            .filter(|(video_id, _)| !emitted.contains(video_id))
            .take(10)
            .map(|(video_id, title)| ModelToolCall {
                id: format!("recipe-save-{video_id}"),
                name: "add_to_sources".into(),
                arguments: json!({
                    "topic_folder": "youtube/playlist",
                    "video_id": video_id,
                    "title": title,
                    "provenance": "source"
                }),
            })
            .collect::<Vec<_>>();
        if !calls.is_empty() {
            return RecipeDecision::ToolCalls(calls);
        }
        RecipeDecision::Complete(format!(
            "Đã xử lý {} video trong playlist.",
            Self::completed_add_calls(transcript)
        ))
    }

    fn progress(&self, params: &Value, transcript: &[ModelMessage]) -> RecipeProgress {
        let max_videos = params
            .get("max_videos")
            .and_then(Value::as_u64)
            .unwrap_or(50)
            .min(50) as usize;
        let listed = Self::list_result(transcript)
            .map(Self::listed_videos)
            .unwrap_or_default()
            .into_iter()
            .take(max_videos)
            .collect::<Vec<_>>();
        let completed = Self::completed_add_calls(transcript);
        let failed_tool = Self::failed_tool_result(transcript);
        let failed_step = failed_tool.map(|failed| {
            if failed.tool_name.as_deref() == Some("yt_list_playlist") {
                0
            } else {
                1
            }
        });
        let list_succeeded = Self::list_result(transcript).is_some() && failed_step != Some(0);
        let current_step = if failed_step.is_some() {
            None
        } else if Self::list_result(transcript).is_none() {
            Some(0)
        } else if completed < listed.len() {
            Some(1)
        } else {
            None
        };
        let _ = params;
        RecipeProgress {
            current_step,
            completed_steps: if list_succeeded && completed == listed.len() {
                vec![0, 1]
            } else if list_succeeded {
                vec![0]
            } else {
                vec![]
            },
            summary: Some(format!("Đã lưu {} trên {} video", completed, listed.len())),
            stop_reason: failed_tool.map(|failed| {
                failed
                    .text
                    .lines()
                    .next()
                    .unwrap_or("recipe tool failed")
                    .to_owned()
            }),
            failed_step,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn playlist_recipe_emits_a_tool_batch_after_listing_without_an_agent_step() {
        let recipe = PlaylistSourcesRecipe;
        let params = json!({"url":"https://www.youtube.com/playlist?list=PL1"});
        let transcript = vec![ModelMessage::tool_result(
            "recipe-list-playlist",
            "yt_list_playlist",
            "1. video_id=vid-1 file_stem=one status=new title=One\n2. video_id=vid-2 file_stem=two status=new title=Two",
        )];

        let RecipeDecision::ToolCalls(calls) = recipe.next(&params, &transcript) else {
            panic!("listing should lead to deterministic source tool calls");
        };
        assert_eq!(calls.len(), 2);
        assert!(calls.iter().all(|call| call.name == "add_to_sources"));
        assert_eq!(calls[0].arguments["topic_folder"], "youtube/playlist");
        assert_eq!(recipe.progress(&params, &transcript).current_step, Some(1));
    }

    #[test]
    fn playlist_recipe_stops_after_a_permanent_tool_error() {
        let recipe = PlaylistSourcesRecipe;
        let params = json!({"url":"https://www.youtube.com/playlist?list=PL1"});
        let transcript = vec![ModelMessage::tool_result(
            "recipe-list-playlist",
            "yt_list_playlist",
            "error YTDLP_FAILED: private playlist",
        )];
        assert!(matches!(
            recipe.next(&params, &transcript),
            RecipeDecision::Failed { failed_step: 0, .. }
        ));
    }
}
