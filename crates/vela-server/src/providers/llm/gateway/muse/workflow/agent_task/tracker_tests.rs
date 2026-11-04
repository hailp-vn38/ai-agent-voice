use super::*;
use serde_json::json;
#[test]
fn agent_task_tracker_rejects_old_turn_and_running_tools() {
    let now = Instant::now();
    let mut tracker = TurnTracker::new(2, now);
    let history = json!({"result":{"status":"completed","chat_events":[
        {"role":"user"}, {"role":"assistant","status":"completed","content":"old [MUSE_TASK_DONE]"}
    ]}});
    tracker.on_history(&history, now);
    assert_eq!(
        tracker.status(now + Duration::from_secs(10)),
        TurnStatus::Working
    );
    let history = json!({"result":{"status":"completed","chat_events":[
        {"role":"user"}, {"role":"user"},
        {"role":"assistant","status":"completed","content":"new [MUSE_TASK_DONE]"},
        {"kind":"tool","status":"running"}
    ]}});
    tracker.on_history(&history, now);
    assert_eq!(
        tracker.status(now + Duration::from_secs(10)),
        TurnStatus::Working
    );
}
#[test]
fn agent_task_tracker_requires_stable_completed_and_excludes_commentary() {
    let now = Instant::now();
    let mut tracker = TurnTracker::new(1, now);
    let mut history = json!({"result":{"status":"completed","chat_events":[
        {"role":"user"}, {"role":"assistant","status":"completed","content":"report"}
    ]}});
    tracker.on_history(&history, now);
    assert_eq!(
        tracker.status(now + Duration::from_secs(4)),
        TurnStatus::Working
    );
    assert_eq!(
        tracker.status(now + Duration::from_secs(5)),
        TurnStatus::Done
    );
    history["result"]["chat_events"][1]["metadata"] = json!({"message_kind":"commentary"});
    tracker.on_history(&history, now + Duration::from_secs(6));
    assert_eq!(
        tracker.status(now + Duration::from_secs(20)),
        TurnStatus::Working
    );
}

#[test]
fn agent_task_tracker_only_required_events_and_real_history_refresh_stall_clock() {
    let now = Instant::now();
    let mut tracker = TurnTracker::new(1, now);
    for kind in ["delta.text_append", "agent.status", "task.updated"] {
        let later = tracker.last_signal_at() + Duration::from_secs(10);
        tracker.on_event(
            &VmStreamEvent {
                raw: json!({"event":kind}),
            },
            later,
        );
        assert_eq!(tracker.last_signal_at(), later);
    }
    let last = tracker.last_signal_at();
    tracker.on_event(
        &VmStreamEvent {
            raw: json!({"event":"heartbeat"}),
        },
        last + Duration::from_secs(10),
    );
    assert_eq!(tracker.last_signal_at(), last);
    let history = json!({"result":{"chat_events":[{"role":"user"},{"role":"assistant","status":"streaming","content":"partial"}]}});
    tracker.on_history(&history, last + Duration::from_secs(20));
    assert_eq!(tracker.last_signal_at(), last + Duration::from_secs(20));
    tracker.on_history(&history, last + Duration::from_secs(30));
    assert_eq!(tracker.last_signal_at(), last + Duration::from_secs(20));
}
