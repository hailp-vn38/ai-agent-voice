use std::{fs, path::PathBuf};

use uuid::Uuid;
use voice_reference_client::scenario::{ScenarioError, ScenarioPlan, ScenarioState};

const RUN_ID: &str = "it_0123456789abcdef01234567";
const REQUIRED_ROLES: &[&str] = &["agent", "template", "vad", "asr", "llm", "tts", "mcp"];

#[test]
fn scenario_plan_requires_the_complete_unique_resource_graph() {
    let raw = b"[scenario]\nkey_prefix = 'qual'\n";

    for invalid_roles in [
        &REQUIRED_ROLES[..REQUIRED_ROLES.len() - 1],
        &["agent", "template", "vad", "asr", "llm", "tts", "tts"][..],
        &["agent", "template", "vad", "asr", "llm", "tts", ""][..],
    ] {
        assert!(matches!(
            ScenarioPlan::materialize(raw, RUN_ID, "qual", invalid_roles),
            Err(ScenarioError::Identity)
        ));
    }

    let plan = ScenarioPlan::materialize(raw, RUN_ID, "qual", REQUIRED_ROLES).unwrap();
    assert_eq!(plan.resource_keys.len(), REQUIRED_ROLES.len());
}

#[test]
fn scenario_state_distinguishes_existing_state_from_other_io_failures() {
    let directory = unique_temp_directory();
    fs::create_dir(&directory).unwrap();
    let state = valid_state();

    let existing = directory.join("existing.json");
    fs::write(&existing, b"occupied").unwrap();
    assert!(matches!(
        state.write_create_new(&existing),
        Err(ScenarioError::StateExists)
    ));

    let unavailable = directory.join("missing-parent").join("state.json");
    assert!(matches!(
        state.write_create_new(&unavailable),
        Err(ScenarioError::Io)
    ));

    fs::remove_dir_all(directory).unwrap();
}

#[cfg(unix)]
#[test]
fn scenario_state_maps_a_non_existing_publish_source_to_io() {
    let directory = unique_temp_directory();
    let moved_directory = directory.with_extension("moved");
    fs::create_dir(&directory).unwrap();
    let destination = directory.join("state.json");
    let temporary = directory.join(".state.json.tmp");
    let mut state = valid_state();
    state.device_id = "x".repeat(32 * 1024 * 1024);

    let writer = std::thread::spawn(move || state.write_create_new(&destination));
    while !temporary.exists() {
        assert!(
            !writer.is_finished(),
            "state writer finished before its temporary sibling was observable"
        );
        std::thread::yield_now();
    }
    fs::rename(&directory, &moved_directory).unwrap();

    assert!(matches!(writer.join().unwrap(), Err(ScenarioError::Io)));
    fs::remove_dir_all(moved_directory).unwrap();
}

fn valid_state() -> ScenarioState {
    let plan = ScenarioPlan::materialize(b"spec", RUN_ID, "qual", REQUIRED_ROLES).unwrap();
    ScenarioState::from_plan(&plan)
}

fn unique_temp_directory() -> PathBuf {
    std::env::temp_dir().join(format!("voice-reference-client-{}", Uuid::new_v4()))
}
