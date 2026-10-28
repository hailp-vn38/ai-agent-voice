use std::collections::VecDeque;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use base64::engine::general_purpose::STANDARD as BASE64;
use base64::Engine;
use futures_util::StreamExt;
use serde_json::{json, Value};

use crate::config::SecretString;
use crate::providers::llm::{
    ChatMessage, LlmEvent, LlmOperation, LlmOutput, LlmProvider, LlmRequest, ReferenceImage,
};

use super::cdp::{BrowserEngine, BrowserPage, CdpError, CookieParam};
use super::config::{MuseConfig, NEW_THREAD_URL};
use super::dom::{
    CHAT_STATE_JS, CLICK_SEND_JS, DENY_APPROVALS_JS, INPUT_EMPTY_JS, LAST_BUBBLE_MEDIA_JS,
    PAGE_STATE_JS,
};
use super::provider::MuseLlmProvider;

const VALID_PNG_BYTES: &[u8] = &[
    0x89, 0x50, 0x4E, 0x47, 0x0D, 0x0A, 0x1A, 0x0A, 0x00, 0x00, 0x00, 0x0D, 0x49, 0x48, 0x44, 0x52,
    0x00, 0x00, 0x00, 0x01, 0x00, 0x00, 0x00, 0x01, 0x08, 0x06, 0x00, 0x00, 0x00, 0x1F, 0x15, 0xC4,
    0x89, 0x00, 0x00, 0x00, 0x0A, 0x49, 0x44, 0x41, 0x54, 0x78, 0x9C, 0x63, 0x00, 0x01, 0x00, 0x00,
    0x05, 0x00, 0x01, 0x0D, 0x0A, 0x2D, 0xB4, 0x00, 0x00, 0x00, 0x00, 0x49, 0x45, 0x4E, 0x44, 0xAE,
    0x42, 0x60, 0x82,
];

const VALID_MP4_BYTES: &[u8] = &[
    0x00, 0x00, 0x00, 0x18, b'f', b't', b'y', b'p', b'm', b'p', b'4', b'2', 0x00, 0x00, 0x00, 0x00,
    b'm', b'p', b'4', b'2', b'i', b's', b'o', b'm',
];

struct FakePageState {
    url: String,
    navigations: Vec<String>,
    attached_files: Vec<String>,
    sent_prompts: Vec<String>,
    denied_count: usize,
    chat_states: VecDeque<Value>,
    last_chat_state: Value,
    blob_payloads: Vec<(String, String, Vec<u8>)>,
}

struct FakeBrowserPage {
    state: Mutex<FakePageState>,
    close_count: AtomicUsize,
}

impl FakeBrowserPage {
    fn new(_context_key: String, chat_states: Vec<Value>) -> Arc<Self> {
        let last_chat_state = chat_states.last().cloned().unwrap_or_else(|| {
            json!({
                "agentCount": 0,
                "lastText": "",
                "generating": false,
                "approval": false,
                "attachments": [],
                "tail": ""
            })
        });
        Arc::new(Self {
            state: Mutex::new(FakePageState {
                url: NEW_THREAD_URL.to_string(),
                navigations: Vec::new(),
                attached_files: Vec::new(),
                sent_prompts: Vec::new(),
                denied_count: 0,
                chat_states: VecDeque::from(chat_states),
                last_chat_state,
                blob_payloads: vec![
                    (
                        "blob:https://muse.ai/img-1".to_string(),
                        "image/png".to_string(),
                        VALID_PNG_BYTES.to_vec(),
                    ),
                    (
                        "blob:https://muse.ai/123".to_string(),
                        "video/mp4".to_string(),
                        VALID_MP4_BYTES.to_vec(),
                    ),
                ],
            }),
            close_count: AtomicUsize::new(0),
        })
    }

    fn push_chat_states(&self, states: Vec<Value>) {
        let mut guard = self.state.lock().unwrap();
        if let Some(last) = states.last().cloned() {
            guard.last_chat_state = last;
        }
        for s in states {
            guard.chat_states.push_back(s);
        }
    }

    fn navigations(&self) -> Vec<String> {
        self.state.lock().unwrap().navigations.clone()
    }

    fn attached_files(&self) -> Vec<String> {
        self.state.lock().unwrap().attached_files.clone()
    }

    fn sent_prompts(&self) -> Vec<String> {
        self.state.lock().unwrap().sent_prompts.clone()
    }

    fn denied_count(&self) -> usize {
        self.state.lock().unwrap().denied_count
    }

    fn close_count(&self) -> usize {
        self.close_count.load(Ordering::SeqCst)
    }
}

#[async_trait::async_trait]
impl BrowserPage for FakeBrowserPage {
    async fn navigate(&self, url: &str) -> Result<(), CdpError> {
        let mut guard = self.state.lock().unwrap();
        guard.url = url.to_string();
        guard.navigations.push(url.to_string());
        Ok(())
    }

    async fn current_url(&self) -> Result<String, CdpError> {
        Ok(self.state.lock().unwrap().url.clone())
    }

    async fn evaluate_json(
        &self,
        expression: &str,
        _await_promise: bool,
        _timeout: Duration,
    ) -> Result<Value, CdpError> {
        let mut guard = self.state.lock().unwrap();
        if expression == PAGE_STATE_JS {
            return Ok(json!({
                "ready": true,
                "hasInput": true,
                "path": "/thread/new",
                "head": "Muse Workspace"
            }));
        }
        if expression == CHAT_STATE_JS {
            let state = if let Some(next) = guard.chat_states.pop_front() {
                guard.last_chat_state = next.clone();
                next
            } else {
                guard.last_chat_state.clone()
            };
            if guard.url == NEW_THREAD_URL
                && state.get("agentCount").and_then(Value::as_u64).unwrap_or(0) > 0
            {
                guard.url = "https://muse.ai/thread/abc".to_string();
            }
            return Ok(state);
        }
        if expression == DENY_APPROVALS_JS {
            guard.denied_count += 1;
            return Ok(json!(["upload approval"]));
        }
        if expression == CLICK_SEND_JS {
            return Ok(json!("clicked"));
        }
        if expression == INPUT_EMPTY_JS {
            return Ok(json!(true));
        }
        if expression == LAST_BUBBLE_MEDIA_JS {
            return Ok(json!({ "links": [], "html": "" }));
        }
        if expression.contains("new File([bytes],") {
            guard.attached_files.push(expression.to_string());
            return Ok(json!({ "ok": true }));
        }
        if expression.contains("HTMLTextAreaElement.prototype, 'value'") {
            if let Some(idx) = expression.find("setter.call(ta, ") {
                let rest = &expression[idx + "setter.call(ta, ".len()..];
                if let Some(end) = rest.find(");\n  ta.dispatchEvent") {
                    let quoted = &rest[..end];
                    if let Ok(parsed) = serde_json::from_str::<String>(quoted) {
                        guard.sent_prompts.push(parsed);
                    }
                }
            }
            return Ok(json!({ "ok": true, "len": 10 }));
        }
        if expression.contains("await fetch(") {
            for (url, mime, bytes) in &guard.blob_payloads {
                if expression.contains(url) {
                    return Ok(json!({
                        "ok": true,
                        "mime": mime,
                        "b64": BASE64.encode(bytes),
                    }));
                }
            }
            return Ok(json!({
                "ok": true,
                "mime": "image/png",
                "b64": BASE64.encode(VALID_PNG_BYTES),
            }));
        }
        if expression.contains("document.body.innerText.includes") {
            return Ok(json!(true));
        }
        Ok(Value::Null)
    }

    async fn press_enter(&self) -> Result<(), CdpError> {
        Ok(())
    }

    async fn close(&self) {
        self.close_count.fetch_add(1, Ordering::SeqCst);
    }
}

struct FakeBrowserEngine {
    pages_queue: Mutex<VecDeque<Arc<FakeBrowserPage>>>,
    opened_pages: Mutex<Vec<Arc<FakeBrowserPage>>>,
    disposed_contexts: Mutex<Vec<String>>,
}

impl FakeBrowserEngine {
    fn new(pages: Vec<Arc<FakeBrowserPage>>) -> Arc<Self> {
        Arc::new(Self {
            pages_queue: Mutex::new(VecDeque::from(pages)),
            opened_pages: Mutex::new(Vec::new()),
            disposed_contexts: Mutex::new(Vec::new()),
        })
    }

    fn opened_pages(&self) -> Vec<Arc<FakeBrowserPage>> {
        self.opened_pages.lock().unwrap().clone()
    }
}

#[async_trait::async_trait]
impl BrowserEngine for FakeBrowserEngine {
    async fn open_page(
        &self,
        context_key: &str,
        _cookies: &[CookieParam],
    ) -> Result<Arc<dyn BrowserPage>, CdpError> {
        let page = self
            .pages_queue
            .lock()
            .unwrap()
            .pop_front()
            .unwrap_or_else(|| FakeBrowserPage::new(context_key.to_string(), Vec::new()));
        self.opened_pages.lock().unwrap().push(Arc::clone(&page));
        Ok(page)
    }

    async fn dispose_context(&self, context_key: &str) {
        self.disposed_contexts
            .lock()
            .unwrap()
            .push(context_key.to_string());
    }
}

fn single_account_config() -> MuseConfig {
    MuseConfig {
        api_key: SecretString::new(
            "hatch_sess=s1; hatch_gw=g1; hatch_vml=v1; hatch_native_auth_device=d1",
        ),
        model: "muse-chat".to_string(),
        timeout_ms: 300_000,
        max_tokens: None,
        context_limit_tokens: None,
        reasoning_effort: None,
        temperature_percent: None,
        tags: Some(vec![
            "chat".to_string(),
            "vision".to_string(),
            "image".to_string(),
            "video".to_string(),
        ]),
        account_concurrency: 1,
        cdp_port: 19222,
        headless: true,
    }
}

#[tokio::test(start_paused = true)]
async fn image_generation_with_reference_images_attaches_files_and_fetches_blob() {
    let page = FakeBrowserPage::new(
        "ctx-1".to_string(),
        vec![
            // base_state before sending prompt
            json!({
                "agentCount": 0,
                "lastText": "",
                "generating": false,
                "approval": false,
                "attachments": [],
                "tail": ""
            }),
            // poll 1: generating with blob attachment ready
            json!({
                "agentCount": 1,
                "lastText": "Here is your image with a red scarf.",
                "generating": false,
                "approval": false,
                "attachments": [{
                    "tid": "hatch-chat-attachment-presentation-image-0",
                    "kind": "image",
                    "src": "blob:https://muse.ai/img-1",
                    "w": 1920,
                    "h": 1080
                }],
                "tail": ""
            }),
        ],
    );
    let engine = FakeBrowserEngine::new(vec![Arc::clone(&page)]);
    let provider = MuseLlmProvider::new_with_engine(single_account_config(), engine).unwrap();

    let ref_img_1 = ReferenceImage {
        bytes: VALID_PNG_BYTES.to_vec(),
        mime_type: "image/png".to_string(),
    };
    let ref_img_2 = ReferenceImage {
        bytes: vec![0xFF, 0xD8, 0xFF, 0xE0],
        mime_type: "image/jpeg".to_string(),
    };

    let output = provider
        .invoke(LlmOperation::ImageGeneration {
            prompt: "Add a red scarf".to_string(),
            width: 1920,
            height: 1080,
            reference_images: vec![ref_img_1, ref_img_2],
            transparent_background: false,
        })
        .await
        .expect("image generation should succeed");

    let attached = page.attached_files();
    assert_eq!(attached.len(), 2);
    assert!(attached[0].contains("input_0.png"));
    assert!(attached[1].contains("input_1.jpg"));

    let prompts = page.sent_prompts();
    assert_eq!(
        prompts,
        vec!["Generate an image (aspect ratio 16:9): Add a red scarf".to_string()]
    );

    assert_eq!(
        output,
        LlmOutput::Media {
            bytes: VALID_PNG_BYTES.to_vec(),
            mime_type: "image/png".to_string(),
        }
    );
}

#[tokio::test(start_paused = true)]
async fn video_generation_denies_approval_card_and_prompts_for_missing_attachment() {
    let mut states = vec![
        // base_state before sending prompt
        json!({
            "agentCount": 0,
            "lastText": "",
            "generating": false,
            "approval": false,
            "attachments": [],
            "tail": ""
        }),
        // Poll 1: inline approval card appears -> driver must auto-deny
        json!({
            "agentCount": 1,
            "lastText": "May I upload the video?",
            "generating": true,
            "approval": true,
            "attachments": [],
            "tail": ""
        }),
    ];
    // Polls during the 90s grace period where agent finished text ("Done!") without attaching
    for _ in 0..160 {
        states.push(json!({
            "agentCount": 1,
            "lastText": "Done!",
            "generating": false,
            "approval": false,
            "attachments": [],
            "tail": ""
        }));
    }
    // Poll after driver sends the follow-up ask-to-attach prompt
    states.push(json!({
        "agentCount": 2,
        "lastText": "Attached the video below.",
        "generating": false,
        "approval": false,
        "attachments": [{
            "tid": "hatch-chat-attachment-presentation-video-0",
            "kind": "video",
            "src": "blob:https://muse.ai/123",
            "w": 1280,
            "h": 720
        }],
        "tail": ""
    }));

    let page = FakeBrowserPage::new("ctx-video".to_string(), states);
    let engine = FakeBrowserEngine::new(vec![Arc::clone(&page)]);
    let provider = MuseLlmProvider::new_with_engine(single_account_config(), engine).unwrap();

    let output = provider
        .invoke(LlmOperation::VideoGeneration {
            prompt: "Ocean waves at sunset".to_string(),
            width: 1280,
            height: 720,
            duration_secs: 5,
            first_frame: None,
        })
        .await
        .expect("video generation should recover after denying approval and asking to attach");

    assert_eq!(page.denied_count(), 1);
    let prompts = page.sent_prompts();
    assert_eq!(
        prompts,
        vec![
            "Generate a video (aspect ratio 16:9, 5 seconds long): Ocean waves at sunset"
                .to_string(),
            "Please attach the final video file here in the chat instead of uploading it."
                .to_string(),
        ]
    );
    assert_eq!(
        output,
        LlmOutput::Media {
            bytes: VALID_MP4_BYTES.to_vec(),
            mime_type: "video/mp4".to_string(),
        }
    );
}

#[tokio::test(start_paused = true)]
async fn chat_streaming_reuses_hot_tab_and_emits_incremental_deltas() {
    let mut turn1_states = vec![
        // base_state
        json!({
            "agentCount": 0,
            "lastText": "",
            "generating": false,
            "approval": false,
            "attachments": [],
            "tail": ""
        }),
        json!({
            "agentCount": 1,
            "lastText": "Hello",
            "generating": true,
            "approval": false,
            "attachments": [],
            "tail": ""
        }),
        json!({
            "agentCount": 1,
            "lastText": "Hello world!",
            "generating": true,
            "approval": false,
            "attachments": [],
            "tail": ""
        }),
    ];
    for _ in 0..5 {
        turn1_states.push(json!({
            "agentCount": 1,
            "lastText": "Hello world!",
            "generating": false,
            "approval": false,
            "attachments": [],
            "tail": ""
        }));
    }

    let page = FakeBrowserPage::new("ctx-chat".to_string(), turn1_states);
    let engine = FakeBrowserEngine::new(vec![Arc::clone(&page)]);
    let provider = MuseLlmProvider::new_with_engine(
        single_account_config(),
        Arc::clone(&engine) as Arc<dyn BrowserEngine>,
    )
    .unwrap();

    // Turn 1: stream and verify incremental deltas
    let mut stream1 = provider
        .stream(LlmRequest::text_turn("Hi Muse".to_string()))
        .await
        .unwrap();

    let mut deltas1 = Vec::new();
    while let Some(ev) = stream1.next().await {
        match ev.unwrap() {
            LlmEvent::TextDelta(d) => deltas1.push(d),
            LlmEvent::Finished => break,
            LlmEvent::ToolCall(_) => panic!("unexpected tool call"),
        }
    }
    assert_eq!(deltas1, vec!["Hello".to_string(), " world!".to_string()]);
    assert_eq!(page.navigations().len(), 1);

    // Prepare Turn 2 states on the same hot page
    let mut turn2_states = vec![
        // verify_hot_tab_live poll_chat_state
        json!({
            "agentCount": 1,
            "lastText": "Hello world!",
            "generating": false,
            "approval": false,
            "attachments": [],
            "tail": ""
        }),
        // base_state before sending followup
        json!({
            "agentCount": 1,
            "lastText": "Hello world!",
            "generating": false,
            "approval": false,
            "attachments": [],
            "tail": ""
        }),
        // new reply bubble
        json!({
            "agentCount": 2,
            "lastText": "4",
            "generating": false,
            "approval": false,
            "attachments": [],
            "tail": ""
        }),
    ];
    for _ in 0..5 {
        turn2_states.push(json!({
            "agentCount": 2,
            "lastText": "4",
            "generating": false,
            "approval": false,
            "attachments": [],
            "tail": ""
        }));
    }
    page.push_chat_states(turn2_states);

    let req2 = LlmRequest {
        messages: vec![
            ChatMessage::User {
                content: "Hi Muse".to_string(),
            },
            ChatMessage::AssistantText {
                content: "Hello world!".to_string(),
            },
            ChatMessage::User {
                content: "What is 2+2?".to_string(),
            },
        ],
        tools: Vec::new(),
    };
    let mut stream2 = provider.stream(req2).await.unwrap();
    let mut deltas2 = Vec::new();
    while let Some(ev) = stream2.next().await {
        match ev.unwrap() {
            LlmEvent::TextDelta(d) => deltas2.push(d),
            LlmEvent::Finished => break,
            LlmEvent::ToolCall(_) => panic!("unexpected tool call"),
        }
    }
    assert_eq!(deltas2, vec!["4".to_string()]);
    assert_eq!(
        engine.opened_pages().len(),
        1,
        "must reuse the same browser tab"
    );
    assert_eq!(
        page.navigations().len(),
        1,
        "must not navigate again when continuing on hot thread"
    );
    assert_eq!(
        page.sent_prompts(),
        vec!["Hi Muse".to_string(), "What is 2+2?".to_string()]
    );
}

#[tokio::test(start_paused = true)]
async fn quota_exhaustion_cools_down_first_account_and_fails_over_to_second_account() {
    let page_acc1 = FakeBrowserPage::new(
        "ctx-acc1".to_string(),
        vec![
            json!({
                "agentCount": 0,
                "lastText": "",
                "generating": false,
                "approval": false,
                "attachments": [],
                "tail": ""
            }),
            json!({
                "agentCount": 1,
                "lastText": "You are out of credits for this billing cycle.",
                "generating": false,
                "approval": false,
                "attachments": [],
                "tail": ""
            }),
        ],
    );

    let mut acc2_states = vec![
        json!({
            "agentCount": 0,
            "lastText": "",
            "generating": false,
            "approval": false,
            "attachments": [],
            "tail": ""
        }),
    ];
    for _ in 0..5 {
        acc2_states.push(json!({
            "agentCount": 1,
            "lastText": "Reply from second account!",
            "generating": false,
            "approval": false,
            "attachments": [],
            "tail": ""
        }));
    }
    let page_acc2 = FakeBrowserPage::new("ctx-acc2".to_string(), acc2_states);

    let engine = FakeBrowserEngine::new(vec![Arc::clone(&page_acc1), Arc::clone(&page_acc2)]);
    let mut cfg = single_account_config();
    cfg.api_key = SecretString::new(
        r#"[
            {"label":"acc-1","cookies":{"hatch_sess":"s1","hatch_gw":"g1","hatch_vml":"v1","hatch_native_auth_device":"d1"}},
            {"label":"acc-2","cookies":{"hatch_sess":"s2","hatch_gw":"g2","hatch_vml":"v2","hatch_native_auth_device":"d2"}}
        ]"#,
    );
    let provider =
        MuseLlmProvider::new_with_engine(cfg, Arc::clone(&engine) as Arc<dyn BrowserEngine>)
            .unwrap();

    let output = provider
        .invoke(LlmOperation::Chat {
            request: LlmRequest::text_turn("Test failover".to_string()),
            reasoning: false,
        })
        .await
        .expect("should fail over to second account on quota error");

    assert_eq!(
        output,
        LlmOutput::Text("Reply from second account!".to_string())
    );
    assert_eq!(engine.opened_pages().len(), 2);
}

#[tokio::test]
async fn cancelled_future_disposes_dirty_tab_via_raii_lease() {
    // First page never finishes generating video
    let dirty_page = FakeBrowserPage::new(
        "ctx-cancel".to_string(),
        vec![
            json!({
                "agentCount": 0,
                "lastText": "",
                "generating": false,
                "approval": false,
                "attachments": [],
                "tail": ""
            }),
            json!({
                "agentCount": 1,
                "lastText": "Working on video...",
                "generating": true,
                "approval": false,
                "attachments": [],
                "tail": ""
            }),
        ],
    );
    let clean_page = FakeBrowserPage::new(
        "ctx-clean".to_string(),
        vec![
            json!({
                "agentCount": 0,
                "lastText": "",
                "generating": false,
                "approval": false,
                "attachments": [],
                "tail": ""
            }),
            json!({
                "agentCount": 1,
                "lastText": "Done",
                "generating": false,
                "approval": false,
                "attachments": [{
                    "tid": "hatch-chat-attachment-presentation-image-0",
                    "kind": "image",
                    "src": "blob:https://muse.ai/img-1",
                    "w": 1024,
                    "h": 1024
                }],
                "tail": ""
            }),
        ],
    );

    let engine = FakeBrowserEngine::new(vec![Arc::clone(&dirty_page), Arc::clone(&clean_page)]);
    let provider = MuseLlmProvider::new_with_engine(
        single_account_config(),
        Arc::clone(&engine) as Arc<dyn BrowserEngine>,
    )
    .unwrap();

    // Cancel the first request after 50ms
    let res = tokio::time::timeout(
        Duration::from_millis(50),
        provider.invoke(LlmOperation::VideoGeneration {
            prompt: "Long video".to_string(),
            width: 1280,
            height: 720,
            duration_secs: 5,
            first_frame: None,
        }),
    )
    .await;
    assert!(res.is_err(), "future should time out and be dropped");

    // Allow background drop worker to process DisposeAborted
    tokio::time::sleep(Duration::from_millis(20)).await;
    assert_eq!(
        dirty_page.close_count(),
        1,
        "TabLease::drop must close dirty tab when future is cancelled"
    );

    // Subsequent request on the same 1-concurrency account must succeed without hanging
    let second = provider
        .invoke(LlmOperation::ImageGeneration {
            prompt: "Quick image".to_string(),
            width: 1024,
            height: 1024,
            reference_images: Vec::new(),
            transparent_background: false,
        })
        .await
        .expect("subsequent request should acquire freed concurrency slot");

    assert!(matches!(second, LlmOutput::Media { .. }));
}
