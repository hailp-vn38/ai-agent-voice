use super::*;

#[test]
fn voice_task_submit_advertises_exact_nine_task_suite_tools_and_matching_reminder_schemas() {
    let submit = VoiceTaskSubmit::unbacked_for_test();
    let defs = submit.tool_definitions();
    let names: Vec<&str> = defs.iter().map(|d| d.name.as_str()).collect();
    assert_eq!(names, TASK_SUITE_TOOL_NAMES);
    assert!(!names.contains(&"task_delete"));

    let reminder_create = defs
        .iter()
        .find(|d| d.name == "reminder_create")
        .expect("reminder_create definition");
    let props = reminder_create.parameters["properties"]
        .as_object()
        .expect("reminder_create properties");
    assert!(props.contains_key("due_at"));
    assert!(props.contains_key("delay_seconds"));
    assert!(props.contains_key("repeat_cron"));
    assert!(props.contains_key("action_kind"));
    assert!(props.contains_key("notes"));

    for tool_name in ["reminder_complete", "reminder_delete"] {
        let def = defs
            .iter()
            .find(|d| d.name == tool_name)
            .expect("reminder mutation definition");
        assert_eq!(def.parameters["required"], serde_json::json!(["reminder_id"]));
    }
}
