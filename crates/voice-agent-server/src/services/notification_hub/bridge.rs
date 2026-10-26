use super::types::{
    NotificationContent, NotificationInput, NotificationSeverity, NotificationSourceKind,
    NotificationTopic,
};
use crate::database::notifications::DueReminder;
use orchestrator_flow_rs::{ScheduleView, TaskEvent, TaskStatus, TaskView};
use serde_json::json;

fn excerpt(input: &str, max_chars: usize) -> String {
    let trimmed = input.trim();
    if trimmed.chars().count() <= max_chars {
        return trimmed.to_owned();
    }
    trimmed.chars().take(max_chars).collect()
}

pub fn from_due_device_reminder(due: &DueReminder) -> NotificationInput {
    let body = due.message.trim();
    let title = format!("Nhắc nhở đến hạn: {}", excerpt(body, 80));
    NotificationInput {
        dedup_key: Some(format!("device_reminder_due:{}", due.reminder_id)),
        content: NotificationContent {
            topic: NotificationTopic::ReminderDue,
            severity: NotificationSeverity::Warning,
            title,
            body: body.to_owned(),
            source_kind: Some(NotificationSourceKind::Schedule),
            source_id: Some(due.reminder_id.clone()),
        },
        metadata: json!({
            "device_row_id": due.device_row_id,
            "device_id": due.device_id,
            "due_at_ms": due.due_at_ms,
            "resource_href": "/devices",
        }),
    }
}

pub fn from_task_reminder_due(schedule: &ScheduleView) -> NotificationInput {
    let run_ts = schedule.last_run_at.unwrap_or(schedule.updated_at);
    let body = schedule
        .notes
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .unwrap_or(schedule.title.as_str());
    NotificationInput {
        dedup_key: Some(format!("task_reminder_due:{}:{run_ts}", schedule.id)),
        content: NotificationContent {
            topic: NotificationTopic::ReminderDue,
            severity: NotificationSeverity::Warning,
            title: format!("Nhắc nhở: {}", excerpt(&schedule.title, 120)),
            body: excerpt(body, 1000),
            source_kind: Some(NotificationSourceKind::Schedule),
            source_id: Some(schedule.id.clone()),
        },
        metadata: json!({
            "schedule_id": schedule.id,
            "resource_href": "/jobs",
        }),
    }
}

pub fn from_task_terminal(task: &TaskView) -> Option<NotificationInput> {
    match task.status {
        TaskStatus::Completed => {
            let summary = task
                .output_preview
                .as_deref()
                .or(task.progress_summary.as_deref())
                .map(str::trim)
                .filter(|s| !s.is_empty())
                .unwrap_or("Tác vụ đã hoàn tất.");

            let is_wiki_flow = task.flow_id == "wiki" || task.agent_id == "wiki_root";
            if is_wiki_flow {
                Some(NotificationInput {
                    dedup_key: Some(format!("wiki_task_completed:{}", task.id)),
                    content: NotificationContent {
                        topic: NotificationTopic::WikiReviewRequired,
                        severity: NotificationSeverity::Info,
                        title: format!("Cập nhật Wiki cần xem: {}", excerpt(&task.title, 120)),
                        body: excerpt(summary, 1000),
                        source_kind: Some(NotificationSourceKind::Wiki),
                        source_id: Some(task.id.clone()),
                    },
                    metadata: json!({
                        "task_id": task.id,
                        "related_wiki_links": task.related_wiki_links,
                        "resource_href": "/knowledge",
                    }),
                })
            } else {
                Some(NotificationInput {
                    dedup_key: Some(format!("task_completed:{}", task.id)),
                    content: NotificationContent {
                        topic: NotificationTopic::TaskCompleted,
                        severity: NotificationSeverity::Success,
                        title: format!("Hoàn tất tác vụ: {}", excerpt(&task.title, 120)),
                        body: excerpt(summary, 1000),
                        source_kind: Some(NotificationSourceKind::Job),
                        source_id: Some(task.id.clone()),
                    },
                    metadata: json!({
                        "flow_id": task.flow_id,
                        "agent_id": task.agent_id,
                        "resource_href": format!("/jobs/{}", task.id),
                    }),
                })
            }
        }
        TaskStatus::Failed => {
            let err_body = task
                .error_message
                .as_deref()
                .or(task.progress_summary.as_deref())
                .map(str::trim)
                .filter(|s| !s.is_empty())
                .unwrap_or("Tác vụ kết thúc với lỗi.");
            Some(NotificationInput {
                dedup_key: Some(format!("task_failed:{}:{}", task.id, task.updated_at)),
                content: NotificationContent {
                    topic: NotificationTopic::TaskFailed,
                    severity: NotificationSeverity::Error,
                    title: format!("Tác vụ thất bại: {}", excerpt(&task.title, 120)),
                    body: excerpt(err_body, 1000),
                    source_kind: Some(NotificationSourceKind::Job),
                    source_id: Some(task.id.clone()),
                },
                metadata: json!({
                    "flow_id": task.flow_id,
                    "agent_id": task.agent_id,
                    "error_code": task.error_code,
                    "resource_href": format!("/jobs/{}", task.id),
                }),
            })
        }
        _ => None,
    }
}

pub fn from_schedule_triggered(schedule: &ScheduleView, spawned_task_id: &str) -> NotificationInput {
    NotificationInput {
        dedup_key: Some(format!(
            "schedule_triggered:{}:{}",
            schedule.id, spawned_task_id
        )),
        content: NotificationContent {
            topic: NotificationTopic::ScheduleTriggered,
            severity: NotificationSeverity::Info,
            title: format!("Lịch định kỳ đã khởi chạy: {}", excerpt(&schedule.title, 120)),
            body: format!(
                "Lịch '{}' đã khởi chạy tác vụ nền #{}.",
                excerpt(&schedule.title, 120),
                spawned_task_id
            ),
            source_kind: Some(NotificationSourceKind::Job),
            source_id: Some(spawned_task_id.to_owned()),
        },
        metadata: json!({
            "schedule_id": schedule.id,
            "spawned_task_id": spawned_task_id,
            "resource_href": format!("/jobs/{spawned_task_id}"),
        }),
    }
}

pub fn from_task_event(event: &TaskEvent) -> Option<NotificationInput> {
    match event {
        TaskEvent::ReminderDue { schedule } => Some(from_task_reminder_due(schedule)),
        TaskEvent::TaskTerminal { task } => from_task_terminal(task),
        TaskEvent::ScheduleTriggered {
            schedule,
            spawned_task_id,
        } => Some(from_schedule_triggered(schedule, spawned_task_id)),
        _ => None,
    }
}

pub fn from_wiki_lint_attention(issue_count: usize, fingerprint: &str, summary: &str) -> NotificationInput {
    NotificationInput {
        dedup_key: Some(format!("wiki_lint:{fingerprint}")),
        content: NotificationContent {
            topic: NotificationTopic::WikiReviewRequired,
            severity: NotificationSeverity::Warning,
            title: format!("Kho tri thức Wiki có {issue_count} mục cần rà soát"),
            body: excerpt(summary, 1000),
            source_kind: Some(NotificationSourceKind::Wiki),
            source_id: Some(fingerprint.to_owned()),
        },
        metadata: json!({
            "issue_count": issue_count,
            "resource_href": "/knowledge",
        }),
    }
}

pub fn from_system_change_request(
    request_id: &str,
    tool_name: &str,
    summary: &str,
) -> NotificationInput {
    NotificationInput {
        dedup_key: Some(format!("system_change_request:{request_id}")),
        content: NotificationContent {
            topic: NotificationTopic::SystemChangeRequest,
            severity: NotificationSeverity::Warning,
            title: format!("Yêu cầu duyệt thay đổi hệ thống ({tool_name})"),
            body: excerpt(summary, 1000),
            source_kind: Some(NotificationSourceKind::System),
            source_id: Some(request_id.to_owned()),
        },
        metadata: json!({
            "tool": tool_name,
            "resource_href": "/system",
        }),
    }
}
