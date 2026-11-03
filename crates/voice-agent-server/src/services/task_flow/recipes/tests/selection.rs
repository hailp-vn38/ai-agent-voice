use super::super::*;
use super::*;

#[test]
fn topic_selection_validation_accepts_only_nonempty_domain_and_kebab_topic() {
    assert!(
        validate_topic_selection(&TopicSelection {
            domain: "technology".into(),
            topic: "machine-learning".into(),
        })
        .is_empty()
    );
    assert_eq!(
        validate_topic_selection(&TopicSelection {
            domain: "  ".into(),
            topic: "machine-learning".into(),
        }),
        vec!["domain must be a non-empty Wiki domain".to_owned()]
    );
    assert_eq!(
        validate_topic_selection(&TopicSelection {
            domain: "technology".into(),
            topic: "Machine Learning".into(),
        }),
        vec!["topic must be a lowercase kebab-case slug".to_owned()]
    );
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
fn domains_created_after_startup_are_accepted_without_restart() {
    let catalog = Arc::new(std::sync::Mutex::new(vec!["science".to_owned()]));
    let source = catalog.clone();
    let recipe =
        PlaylistSourcesRecipe::with_domain_source(Arc::new(move || source.lock().unwrap().clone()));
    let params = json!({
        "url":"https://www.youtube.com/playlist?list=PL1",
        "domain":"history"
    });
    assert!(recipe.validate_params(&params).is_err());

    catalog.lock().unwrap().push("history".into());
    assert_eq!(
        recipe.metadata().params_schema["properties"]["domain"]["enum"][1],
        "history"
    );
    assert_eq!(
        recipe.validate_params(&params).unwrap()["domain"],
        "history"
    );
}
