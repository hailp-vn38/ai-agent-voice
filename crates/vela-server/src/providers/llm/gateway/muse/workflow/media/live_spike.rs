//! Spike với tài khoản Muse thật cho Giai đoạn 4 của `docs/agent-job-durability-implementation-guide.md`.
//! Tốn quota video, nên luôn `#[ignore]` và phải đặt `MUSE_SPIKE_CONFIRM=spend-quota`.
//! Cách chạy và cách đọc kết quả: `docs/providers/muse-durability-spike.md`.

use std::sync::Arc;
use std::time::Duration;

use serde::Serialize;
use tokio::time::Instant;

use super::candidates::{
    extract_candidates_from_stream_and_history, select_effective_ready_attachments,
};
use super::{
    MediaBatch, SERVICE_DAEMON, count_remote_user_turns, run_media_turn_once, total_duration_secs,
};
use crate::providers::llm::gateway::muse::pool::{SharedMusePool, job_entry, release_job_session};
use crate::providers::llm::gateway::muse::workflow::{
    MediaKind, WorkflowFailure, extract_history_turn_events, extract_turn_assistant_messages,
};
use crate::providers::llm::{LlmOutput, MediaPart};

const MEDIA_DEADLINE: Duration = Duration::from_secs(900);
const MAX_PROBE_READ_BYTES: usize = 256 * 1024;

/// Trạng thái một session đọc lại bằng kết nối mới, không có stream của lượt gửi.
#[derive(Debug, Default, PartialEq, Eq, Serialize)]
pub(crate) struct SessionProbe {
    pub acquire_error: Option<String>,
    pub history_status: Option<u32>,
    pub user_turns: Option<u32>,
    /// Số file sẵn sàng trong history khi `stream_presentations` rỗng.
    pub ready_paths: Vec<String>,
    pub read_path: Option<String>,
    pub read_bytes: Option<usize>,
    pub read_error: Option<String>,
}

/// Mở lại `session_id` trên `pool`, đọc history rồi thử `/fs/read` file đã biết hoặc file mới nhất.
pub(crate) async fn probe_session(
    pool: &Arc<SharedMusePool>,
    session_id: &str,
    known_path: Option<&str>,
) -> SessionProbe {
    let mut probe = SessionProbe::default();
    let mut lease = match pool.acquire_existing_session(session_id.to_string()).await {
        Ok(lease) => lease,
        Err((kind, reason)) => {
            probe.acquire_error = Some(format!("{kind:?}: {reason}"));
            return probe;
        }
    };
    lease.retain_session();
    probe.user_turns = count_remote_user_turns(&mut lease).await;
    let history = lease
        .client()
        .send_request(
            "GET",
            &format!("/chat/history?session_id={session_id}"),
            None,
            SERVICE_DAEMON,
            Duration::from_secs(15),
            false,
        )
        .await;
    if let Ok(resp) = history {
        probe.history_status = Some(resp.status);
        if let Ok(json) = resp.json() {
            let events = extract_history_turn_events(&json);
            let bubbles = extract_turn_assistant_messages(&events);
            let candidates = extract_candidates_from_stream_and_history(
                MediaKind::Video,
                &[],
                &events,
                &bubbles,
            );
            probe.ready_paths = select_effective_ready_attachments(&candidates)
                .iter()
                .map(|candidate| candidate.path.clone())
                .collect();
        }
    }
    let path = known_path
        .map(str::to_string)
        .or_else(|| probe.ready_paths.first().cloned());
    if let Some(path) = path {
        match lease.client().read_file(&path, MAX_PROBE_READ_BYTES).await {
            Ok(bytes) => probe.read_bytes = Some(bytes.len()),
            Err(err) => probe.read_error = Some(format!("{err:?}")),
        }
        probe.read_path = Some(path);
    }
    lease.finish_clean(None);
    probe
}

#[derive(Debug, Serialize)]
struct Checkpoint {
    minutes_after_disconnect: u64,
    owner: SessionProbe,
    other_account: Option<SessionProbe>,
}

#[derive(Debug, Serialize)]
struct TurnOutcome {
    requested_secs: u32,
    result: String,
    actual_secs: Option<u32>,
    parts: usize,
}

#[derive(Debug, Default, Serialize)]
struct Report {
    session_id: Option<String>,
    abandoned_turn: Option<String>,
    checkpoints: Vec<Checkpoint>,
    clip_limit_turns: Vec<TurnOutcome>,
    /// `false` nghĩa là còn session trên Muse, phải dọn tay.
    session_deleted: Option<bool>,
}

fn describe_turn(requested_secs: u32, outcome: Result<LlmOutput, WorkflowFailure>) -> TurnOutcome {
    match outcome {
        Ok(LlmOutput::Media {
            bytes,
            mime_type,
            extra_parts,
            ..
        }) => {
            let mut parts = vec![MediaPart { bytes, mime_type }];
            parts.extend(extra_parts);
            if let Ok(dir) = std::env::var("MUSE_SPIKE_MEDIA_DIR") {
                for (index, part) in parts.iter().enumerate() {
                    let path = format!("{dir}/turn-{requested_secs}s-part-{index}.mp4");
                    std::fs::write(path, &part.bytes).expect("write spike video");
                }
            }
            TurnOutcome {
                requested_secs,
                result: "ok".into(),
                actual_secs: total_duration_secs(&parts),
                parts: parts.len(),
            }
        }
        Ok(_) => TurnOutcome {
            requested_secs,
            result: "unexpected non-media output".into(),
            actual_secs: None,
            parts: 0,
        },
        Err(failure) => TurnOutcome {
            requested_secs,
            result: format!("{failure:?}"),
            actual_secs: None,
            parts: 0,
        },
    }
}

fn required_env(name: &str) -> String {
    std::env::var(name).unwrap_or_else(|_| panic!("{name} must be set for the live spike"))
}

fn live_pool(account: &str, credential: &str) -> Arc<SharedMusePool> {
    SharedMusePool::for_gateway_lease(&reqwest::Client::new(), account, credential, None)
}

/// Gửi video, bỏ ngang và ngắt kết nối, rồi đọc lại theo các mốc: session, account, `session_id`
/// client tạo, history không có stream và `/fs/read` sau khi đứt.
async fn run_abandon_stage(report: &mut Report, owner: &Arc<SharedMusePool>, job_key: &str) {
    let abandon_after = std::env::var("MUSE_SPIKE_ABANDON_AFTER_SECS")
        .ok()
        .and_then(|raw| raw.parse().ok())
        .unwrap_or(45);
    let checkpoints: Vec<u64> = std::env::var("MUSE_SPIKE_CHECKPOINT_MINUTES")
        .unwrap_or_else(|_| "1,5,15".into())
        .split(',')
        .filter_map(|raw| raw.trim().parse().ok())
        .collect();
    let other = std::env::var("MUSE_SPIKE_CREDENTIAL_B")
        .ok()
        .map(|credential| live_pool("muse-spike-b", &credential));

    let job = job_entry(job_key);
    let started = std::sync::atomic::AtomicBool::new(false);
    let batches = [MediaBatch {
        prompt: "A 8 second video of a paper boat on a calm lake, one continuous scene.".into(),
        target_secs: Some(8),
    }];
    let turn = run_media_turn_once(
        owner,
        MediaKind::Video,
        &batches,
        &[],
        false,
        Some(Arc::clone(&job)),
        None,
        Instant::now() + MEDIA_DEADLINE,
        &started,
        None,
        None,
    );
    report.abandoned_turn = Some(
        match tokio::time::timeout(Duration::from_secs(abandon_after), turn).await {
            Ok(outcome) => {
                format!(
                    "finished before abandoning: {:?}",
                    describe_turn(8, outcome)
                )
            }
            Err(_) => format!("abandoned after {abandon_after}s"),
        },
    );
    let session_id = job
        .lock()
        .await
        .as_ref()
        .map(|session| session.session_id.clone())
        .expect("the first call must record its session before sending");
    report.session_id = Some(session_id.clone());

    // Ngắt WebSocket: lần đọc sau phải tự mở kết nối mới.
    let lease = owner
        .acquire_existing_session(session_id.clone())
        .await
        .expect("reacquire owner account");
    lease.client().close().await;
    drop(lease);

    let disconnected_at = Instant::now();
    let mut known_path = None;
    for minutes in checkpoints {
        tokio::time::sleep_until(disconnected_at + Duration::from_secs(minutes * 60)).await;
        let probe = probe_session(owner, &session_id, known_path.as_deref()).await;
        if let Some(path) = &probe.read_path {
            known_path = Some(path.clone());
        }
        let other_account = match &other {
            Some(pool) => Some(probe_session(pool, &session_id, known_path.as_deref()).await),
            None => None,
        };
        report.checkpoints.push(Checkpoint {
            minutes_after_disconnect: minutes,
            owner: probe,
            other_account,
        });
    }
}

/// Lô 60s (6 clip) rồi xin thêm trong cùng session: giới hạn 6 clip theo lượt hay theo session,
/// và lượt nhắc có dùng lại clip cũ không (so nội dung file bằng mắt theo runbook).
async fn run_clip_limit_stage(report: &mut Report, owner: &Arc<SharedMusePool>, job_key: &str) {
    let job = job_entry(job_key);
    for (requested_secs, prompt) in [
        (
            60,
            "A 60 second documentary about paper boats, six scenes of 10 seconds each.",
        ),
        (
            20,
            "Continue the same documentary with two more 10 second scenes about rivers.",
        ),
    ] {
        let started = std::sync::atomic::AtomicBool::new(false);
        let batches = [MediaBatch {
            prompt: prompt.into(),
            target_secs: Some(requested_secs),
        }];
        let outcome = run_media_turn_once(
            owner,
            MediaKind::Video,
            &batches,
            &[],
            false,
            Some(Arc::clone(&job)),
            None,
            Instant::now() + MEDIA_DEADLINE,
            &started,
            None,
            None,
        )
        .await;
        report
            .clip_limit_turns
            .push(describe_turn(requested_secs, outcome));
    }
}

#[tokio::test(flavor = "multi_thread")]
#[ignore = "spends real Muse video quota; see docs/providers/muse-durability-spike.md"]
async fn muse_durability_live_spike() {
    assert_eq!(
        required_env("MUSE_SPIKE_CONFIRM"),
        "spend-quota",
        "MUSE_SPIKE_CONFIRM must acknowledge the quota cost"
    );
    let owner = live_pool("muse-spike-a", &required_env("MUSE_SPIKE_CREDENTIAL_A"));
    let stages = std::env::var("MUSE_SPIKE_STAGES").unwrap_or_else(|_| "abandon".into());
    let output = std::env::var("MUSE_SPIKE_OUTPUT")
        .unwrap_or_else(|_| "muse-durability-spike-result.json".into());
    let job_key = format!("muse-spike-{}", uuid::Uuid::new_v4());

    let mut report = Report::default();
    if stages.contains("abandon") {
        run_abandon_stage(&mut report, &owner, &job_key).await;
    }
    if stages.contains("clips") {
        run_clip_limit_stage(&mut report, &owner, &job_key).await;
    }
    report.session_deleted = Some(release_job_session(&job_key).await.is_some());

    let json = serde_json::to_string_pretty(&report).expect("serialize spike report");
    std::fs::write(&output, &json).expect("write spike report");
    println!("{json}");
}
