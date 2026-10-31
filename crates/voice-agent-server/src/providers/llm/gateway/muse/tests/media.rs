use std::collections::VecDeque;
use std::sync::Arc;

use serde_json::json;

use super::super::workflow::decode_uncompressed_or_simple_png;
use super::harness::{
    FakeMuseVmTransport, FakeStreamTurn, FakeVmAccount, SINGLE_COOKIE, VALID_MP4_BYTES,
    VALID_PNG_BYTES, build_gateway_muse_provider, make_test_rgba_png,
};
use crate::providers::llm::{LlmOperation, LlmOutput, LlmProvider, MediaPart, ReferenceImage};

#[tokio::test]
async fn image_generation_attaches_reference_images_reads_fs_and_removes_background() {
    let raw_png = make_test_rgba_png(12, 12, 3..9);

    let turn = FakeStreamTurn::media_reply(
        vec![json!({
            "id": "pres-img-1",
            "kind": "image",
            "image_path": "sandbox:///home/user/generated_subject.png",
        })],
        vec![vec![
            json!({ "role": "user", "display_text": "prompt" }),
            json!({
                "role": "assistant",
                "status": "completed",
                "display_text": "Here is your sticker [ALL_MEDIA_ATTACHED]"
            }),
        ]],
        vec![],
    );

    let acc = FakeVmAccount::new(vec![turn])
        .with_file("/home/user/generated_subject.png", raw_png.clone());
    let transport = FakeMuseVmTransport::new(vec![Arc::clone(&acc)]);
    let provider = build_gateway_muse_provider(&[SINGLE_COOKIE], transport).await;

    let output = provider
        .invoke(LlmOperation::ImageGeneration {
            prompt: "A green crystal badge".to_string(),
            width: 1024,
            height: 1024,
            reference_images: vec![
                ReferenceImage {
                    bytes: VALID_PNG_BYTES.to_vec(),
                    mime_type: "image/png".to_string(),
                },
                ReferenceImage {
                    bytes: VALID_PNG_BYTES.to_vec(),
                    mime_type: "image/jpeg".to_string(),
                },
            ],
            transparent_background: true,
        })
        .await
        .expect("image generation succeeds");

    let reqs = acc.stream_requests();
    assert_eq!(reqs.len(), 1);
    let items = reqs[0]["items"].as_array().expect("items array");
    assert_eq!(items.len(), 3, "2 reference images + 1 text item");
    assert_eq!(items[0]["type"].as_str(), Some("image"));
    assert_eq!(items[1]["type"].as_str(), Some("image"));
    assert_eq!(items[2]["type"].as_str(), Some("text"));

    let LlmOutput::Media {
        bytes,
        mime_type,
        assistant_message,
        ..
    } = output
    else {
        panic!("expected LlmOutput::Media");
    };
    assert_eq!(assistant_message.as_deref(), Some("Here is your sticker"));
    assert_eq!(mime_type, "image/png");
    let (_w, _h, rgba) =
        decode_uncompressed_or_simple_png(&bytes).expect("decode output rgba png");
    assert_eq!(rgba[3], 0, "background removed to alpha 0");

    // Allow spawned ephemeral session cleanup task to run
    tokio::task::yield_now().await;
    assert_eq!(
        acc.deleted_sessions().len(),
        1,
        "ephemeral media session must be deleted via /session/delete"
    );
}

#[tokio::test]
async fn video_generation_harvests_all_multi_clip_paths_via_fs_read_and_assistant_message() {
    let clip1_bytes = [VALID_MP4_BYTES, &[1u8; 4]].concat();
    let clip2_bytes = [VALID_MP4_BYTES, &[2u8; 4]].concat();
    let clip3_bytes = [VALID_MP4_BYTES, &[3u8; 4]].concat();

    // Poll 1: tool_group is still running ("in_progress") with only 2 clips -> workflow must wait!
    let poll_running = vec![
        json!({ "role": "user", "display_text": "prompt" }),
        json!({ "role": "assistant", "display_text": "Intro: 3 scenes (30s total)" }),
        json!({
            "event_name": "tool_group",
            "payload": { "status": "in_progress", "data": { "display_state": "working" } }
        }),
        json!({
            "event_name": "delta.presentation",
            "payload": {
                "kind": "video",
                "data": {
                    "videos": [
                        { "path": "sandbox:///home/user/clip_1.mp4" },
                        { "path": "sandbox:///home/user/clip_2.mp4" }
                    ]
                }
            }
        }),
    ];

    // Poll 2: tool_group completed, all 3 clips attached, and [ALL_MEDIA_ATTACHED] marker present
    let poll_completed = vec![
        json!({ "role": "user", "display_text": "prompt" }),
        json!({ "role": "assistant", "display_text": "Intro: 3 scenes (30s total)" }),
        json!({ "role": "assistant", "display_text": "Scene 1: Persian Gulf overview (10s)" }),
        json!({ "role": "assistant", "display_text": "Scene 2: Naval formation (10s)" }),
        json!({
            "role": "assistant",
            "status": "completed",
            "display_text": "Scene 3: Diplomatic summary (10s) [ALL_MEDIA_ATTACHED]"
        }),
        json!({
            "event_name": "delta.presentation",
            "payload": {
                "kind": "video",
                "data": {
                    "videos": [
                        { "path": "sandbox:///home/user/clip_1.mp4" },
                        { "path": "sandbox:///home/user/clip_2.mp4" },
                        { "path": "sandbox:///home/user/clip_3.mp4" }
                    ]
                }
            }
        }),
    ];

    let turn = FakeStreamTurn::media_reply(
        vec![],
        vec![poll_running, poll_completed],
        vec![json!({
            "event": "approval.requested",
            "payload": {
                "approval_id": "app-bash-1",
                "command": "curl https://example.com/upload"
            }
        })],
    );

    let acc = FakeVmAccount::new(vec![turn])
        .with_file("/home/user/clip_1.mp4", clip1_bytes.clone())
        .with_file("/home/user/clip_2.mp4", clip2_bytes.clone())
        .with_file("/home/user/clip_3.mp4", clip3_bytes.clone());
    let transport = FakeMuseVmTransport::new(vec![Arc::clone(&acc)]);
    let provider = build_gateway_muse_provider(&[SINGLE_COOKIE], transport).await;

    let output = provider
        .invoke(LlmOperation::VideoGeneration {
            prompt: "Iran war overview in 3 scenes".to_string(),
            width: 1280,
            height: 720,
            duration_secs: 30,
            first_frame: None,
        })
        .await
        .expect("multi-clip video generation succeeds");

    assert_eq!(
        acc.decided_approvals(),
        vec![("app-bash-1".to_string(), "deny".to_string())],
        "external curl/bash approval must be auto-denied"
    );

    assert_eq!(
        output,
        LlmOutput::from_media_parts(
            vec![
                MediaPart {
                    bytes: clip1_bytes,
                    mime_type: "video/mp4".to_string(),
                },
                MediaPart {
                    bytes: clip2_bytes,
                    mime_type: "video/mp4".to_string(),
                },
                MediaPart {
                    bytes: clip3_bytes,
                    mime_type: "video/mp4".to_string(),
                },
            ],
            Some(
                "Intro: 3 scenes (30s total)\n\nScene 1: Persian Gulf overview (10s)\n\nScene 2: Naval formation (10s)\n\nScene 3: Diplomatic summary (10s)"
                    .to_string()
            ),
        )
        .unwrap()
    );
}

#[tokio::test]
async fn video_generation_selects_final_merged_group_when_raw_clips_precede_merged_output() {
    let raw1 = [VALID_MP4_BYTES, &[0xA1; 4]].concat();
    let raw2 = [VALID_MP4_BYTES, &[0xA2; 4]].concat();
    let merged = [VALID_MP4_BYTES, &[0xFF; 8]].concat();

    let history = vec![
        json!({ "role": "user", "display_text": "prompt" }),
        json!({
            "event_name": "delta.presentation",
            "payload": {
                "kind": "video",
                "data": {
                    "videos": [
                        { "path": "/home/user/raw_1.mp4" },
                        { "path": "/home/user/raw_2.mp4" }
                    ]
                }
            }
        }),
        json!({
            "event_name": "delta.presentation",
            "payload": {
                "kind": "video",
                "video_path": "/home/user/final_merged.mp4"
            }
        }),
        json!({
            "role": "assistant",
            "status": "completed",
            "display_text": "Merged 20s video ready [ALL_MEDIA_ATTACHED]"
        }),
    ];

    let turn = FakeStreamTurn::media_reply(vec![], vec![history], vec![]);
    let acc = FakeVmAccount::new(vec![turn])
        .with_file("/home/user/raw_1.mp4", raw1)
        .with_file("/home/user/raw_2.mp4", raw2)
        .with_file("/home/user/final_merged.mp4", merged.clone());
    let transport = FakeMuseVmTransport::new(vec![acc]);
    let provider = build_gateway_muse_provider(&[SINGLE_COOKIE], transport).await;

    let output = provider
        .invoke(LlmOperation::VideoGeneration {
            prompt: "20s cinematic trailer".to_string(),
            width: 1280,
            height: 720,
            duration_secs: 20,
            first_frame: None,
        })
        .await
        .expect("merged video generation succeeds");

    assert_eq!(
        output,
        LlmOutput::from_media_parts(
            vec![MediaPart {
                bytes: merged,
                mime_type: "video/mp4".to_string(),
            }],
            Some("Merged 20s video ready".to_string()),
        )
        .unwrap()
    );
}

#[tokio::test]
async fn image_generation_completes_from_history_presentation_without_done_marker_and_reads_relative_workspace_path()
 {
    let webp_bytes = [
        b"RIFF".as_slice(),
        &[0x24, 0x00, 0x00, 0x00],
        b"WEBP",
        b"VP8 ",
        &[0x10, 0x00, 0x00, 0x00, 0x9D, 0x01, 0x2A, 0x80, 0x04, 0x00, 0x08],
    ]
    .concat();
    let rel_path =
        "workspace/imagine_media/media-generation-generate-an-image-aspect-ratio-0-de718b19.webp";
    let sandbox_path = format!("sandbox://{rel_path}");

    let turn = FakeStreamTurn {
        status: 200,
        assigned_session_id: Some("5c21af1d-7235-4e0e-b5fc-705e0105f21b".to_string()),
        events: vec![json!({
            "type": "event",
            "event": "sessions.updated",
            "payload": {
                "change": "updated",
                "session": {
                    "session_id": "5c21af1d-7235-4e0e-b5fc-705e0105f21b",
                    "status": "running"
                }
            }
        })],
        history_polls: VecDeque::from(vec![vec![
            json!({
                "event_name": "message.user",
                "payload": {
                    "role": "user",
                    "display_text": "Generate an image (aspect ratio 9:16): Cinematic fantasy poster"
                }
            }),
            json!({
                "event_name": "message.assistant",
                "display_text": "Done — modern xianxia poster, 9:16, with the cultivator on the rain-slick rooftop.",
                "payload": {
                    "role": "assistant",
                    "status": "completed",
                    "display_text": "Done — modern xianxia poster, 9:16, with the cultivator on the rain-slick rooftop."
                }
            }),
            json!({
                "event_name": "delta.presentation",
                "payload": {
                    "id": "widget-8039231a",
                    "kind": "image",
                    "image_path": rel_path,
                    "fallback_text": format!("![Modern cultivation xianxia poster]({sandbox_path})"),
                    "data": {
                        "fallback_text": format!("![Modern cultivation xianxia poster]({sandbox_path})"),
                        "images": [{
                            "byte_len": webp_bytes.len(),
                            "width": 1152,
                            "height": 2048,
                            "mime": "image/webp",
                            "missing": false,
                            "path": sandbox_path
                        }]
                    }
                }
            }),
        ]]),
    };

    let acc = FakeVmAccount::new(vec![turn]).with_file(rel_path, webp_bytes.clone());
    let transport = FakeMuseVmTransport::new(vec![Arc::clone(&acc)]);
    let provider = build_gateway_muse_provider(&[SINGLE_COOKIE], transport).await;

    let output = provider
        .invoke(LlmOperation::ImageGeneration {
            prompt: "Cinematic fantasy poster illustration of modern cultivation / xianxia"
                .to_string(),
            width: 1024,
            height: 1792,
            reference_images: Vec::new(),
            transparent_background: false,
        })
        .await
        .expect("image generation succeeds without /home/user prefix or [ALL_MEDIA_ATTACHED]");

    assert_eq!(
        output,
        LlmOutput::Media {
            bytes: webp_bytes,
            mime_type: "image/webp".to_string(),
            extra_parts: Vec::new(),
            assistant_message: Some(
                "Done — modern xianxia poster, 9:16, with the cultivator on the rain-slick rooftop."
                    .to_string()
            ),
        }
    );
}

#[tokio::test]
async fn video_generation_waits_across_commentary_and_message_to_presentation_gap_for_50s_merged_video()
 {
    let merged_bytes = [VALID_MP4_BYTES, &[0xCC; 16]].concat();
    let final_rel_path = "workspace/imagine_media/chelsea-vs-mu-50s-final.mp4";
    let final_sandbox_path = format!("sandbox://{final_rel_path}");

    // Poll 1: Muse emits an initial `message.assistant` with `metadata.message_kind = "commentary"`
    // and `status = "completed"` while generating the 5 sequential 10s clips.
    // Workflow must NOT treat this commentary bubble as final completion or send a follow-up nudge.
    let poll_commentary = vec![
        json!({
            "event_name": "message.user",
            "payload": {
                "role": "user",
                "display_text": "Generate a video (aspect ratio 9:16, target total duration ~50s): Chelsea vs MU"
            }
        }),
        json!({
            "event_name": "message.assistant",
            "status": "completed",
            "display_text": "Generating your 50-second explainer as 5 clips (~10 seconds each), then stitching them with burned-in Vietnamese captions.",
            "metadata": {
                "message_kind": "commentary",
                "recipient": "commentary"
            },
            "payload": {
                "role": "assistant",
                "status": "completed",
                "display_text": "Generating your 50-second explainer as 5 clips (~10 seconds each), then stitching them with burned-in Vietnamese captions.",
                "metadata": {
                    "message_kind": "commentary",
                    "recipient": "commentary"
                }
            }
        }),
    ];

    // Poll 2: Muse records `ev[1]` (`message.assistant`, `status = "completed"`, `message_kind = "message"`)
    // ~955ms BEFORE `ev[2]` (`delta.presentation`) appears in `/chat/history`.
    // Workflow must wait across this ~1s gap without sending a follow-up nudge that would overwrite the turn.
    let poll_message_before_presentation = vec![
        json!({
            "event_name": "message.user",
            "payload": {
                "role": "user",
                "display_text": "Generate a video (aspect ratio 9:16, target total duration ~50s): Chelsea vs MU"
            }
        }),
        json!({
            "event_name": "message.assistant",
            "status": "completed",
            "display_text": "Đã ghép xong 5 beat thành 1 video duy nhất — đã kiểm chứng: 1080x1920 (9:16), H.264 + AAC, 50.07 giây.",
            "metadata": {
                "message_kind": "message",
                "recipient": "all"
            },
            "payload": {
                "role": "assistant",
                "status": "completed",
                "display_text": "Đã ghép xong 5 beat thành 1 video duy nhất — đã kiểm chứng: 1080x1920 (9:16), H.264 + AAC, 50.07 giây.",
                "metadata": {
                    "message_kind": "message",
                    "recipient": "all"
                }
            }
        }),
    ];

    // Poll 3: `delta.presentation` and final `message.assistant` with `[ALL_MEDIA_ATTACHED]` arrive.
    let poll_completed = vec![
        json!({
            "event_name": "message.user",
            "payload": {
                "role": "user",
                "display_text": "Generate a video (aspect ratio 9:16, target total duration ~50s): Chelsea vs MU"
            }
        }),
        json!({
            "event_name": "message.assistant",
            "status": "completed",
            "display_text": "Đã ghép xong 5 beat thành 1 video duy nhất — đã kiểm chứng: 1080x1920 (9:16), H.264 + AAC, 50.07 giây.",
            "metadata": {
                "message_kind": "message",
                "recipient": "all"
            },
            "payload": {
                "role": "assistant",
                "status": "completed",
                "display_text": "Đã ghép xong 5 beat thành 1 video duy nhất — đã kiểm chứng: 1080x1920 (9:16), H.264 + AAC, 50.07 giây."
            }
        }),
        json!({
            "event_name": "delta.presentation",
            "payload": {
                "id": "widget-da9e29b6",
                "kind": "video",
                "video_path": final_rel_path,
                "fallback_text": format!("![Chelsea vs MU — 50s Preview Video (9:16)]({final_sandbox_path})"),
                "data": {
                    "fallback_text": format!("![Chelsea vs MU — 50s Preview Video (9:16)]({final_sandbox_path})"),
                    "videos": [{
                        "byte_len": merged_bytes.len(),
                        "duration_secs": 50.066667,
                        "width": 1080,
                        "height": 1920,
                        "mime": "video/mp4",
                        "missing": false,
                        "path": final_sandbox_path
                    }]
                }
            }
        }),
        json!({
            "event_name": "message.assistant",
            "status": "completed",
            "display_text": "Hoàn tất video Chelsea vs MU — 50 giây, dọc 9:16, phụ đề tiếng Việt đốt trực tiếp theo đúng kịch bản, không thêm dữ kiện ngoài brief. [ALL_MEDIA_ATTACHED]",
            "metadata": {
                "message_kind": "message",
                "recipient": "all"
            },
            "payload": {
                "role": "assistant",
                "status": "completed",
                "display_text": "Hoàn tất video Chelsea vs MU — 50 giây, dọc 9:16, phụ đề tiếng Việt đốt trực tiếp theo đúng kịch bản, không thêm dữ kiện ngoài brief. [ALL_MEDIA_ATTACHED]"
            }
        }),
    ];

    let turn = FakeStreamTurn::media_reply(
        vec![],
        vec![
            poll_commentary,
            poll_message_before_presentation,
            poll_completed,
        ],
        vec![],
    );

    let acc = FakeVmAccount::new(vec![turn]).with_file(final_rel_path, merged_bytes.clone());
    let transport = FakeMuseVmTransport::new(vec![Arc::clone(&acc)]);
    let provider = build_gateway_muse_provider(&[SINGLE_COOKIE], transport).await;

    let output = provider
        .invoke(LlmOperation::VideoGeneration {
            prompt: "Chelsea vs MU 50s preview".to_string(),
            width: 1080,
            height: 1920,
            duration_secs: 50,
            first_frame: None,
        })
        .await
        .expect("50s merged video generation succeeds without premature nudge");

    assert_eq!(
        acc.stream_requests().len(),
        1,
        "must not send a premature follow-up stream request during commentary or the 1s presentation gap"
    );

    assert_eq!(
        output,
        LlmOutput::from_media_parts(
            vec![MediaPart {
                bytes: merged_bytes,
                mime_type: "video/mp4".to_string(),
            }],
            Some(
                "Đã ghép xong 5 beat thành 1 video duy nhất — đã kiểm chứng: 1080x1920 (9:16), H.264 + AAC, 50.07 giây.\n\nHoàn tất video Chelsea vs MU — 50 giây, dọc 9:16, phụ đề tiếng Việt đốt trực tiếp theo đúng kịch bản, không thêm dữ kiện ngoài brief."
                    .to_string()
            ),
        )
        .unwrap()
    );
}
