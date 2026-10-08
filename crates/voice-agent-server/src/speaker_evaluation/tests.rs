use super::*;

fn genuine(identity: &str) -> GroundTruth {
    GroundTruth::Genuine {
        identity: identity.into(),
    }
}

fn matched(identity: &str) -> TrialOutcome {
    TrialOutcome::Match {
        identity: identity.into(),
    }
}

fn trial(
    sample: &str,
    session: &str,
    path: Path,
    ground_truth: GroundTruth,
    outcome: TrialOutcome,
) -> Trial {
    Trial {
        sample_code: sample.into(),
        session_code: session.into(),
        path,
        ground_truth,
        outcome,
    }
}

fn protocol() -> ProtocolPin {
    ProtocolPin {
        corpus_version: "corpus-v1".into(),
        protocol_version: "pilot-v1".into(),
        embedding_space: "speaker:campplus".into(),
        voiceprint_revision: 7,
        catalog_revision: 3,
        candidate_set_id: "set-001".into(),
        holdout_epoch: 4,
        tuning_epoch: 4,
        enrollment_sessions: BTreeSet::from(["enroll-s01".to_string()]),
    }
}

fn scope() -> ScopePin {
    ScopePin {
        machine: "pilot-host-1".into(),
        model_revision: "campplus-v1".into(),
        threads: 4,
        worker: "speaker-0".into(),
        queue_depth: 1,
        workload: "pilot-1n-1to1".into(),
        audio_conditions: "quiet-office".into(),
    }
}

fn passing_latency() -> LatencySamples {
    LatencySamples {
        inference_ms: vec![100.0; 20],
        gate_wait_ms: vec![300.0; 20],
        queue_ms: vec![5.0; 20],
        asr_ms: vec![80.0; 20],
        barrier_ms: vec![10.0; 20],
    }
}

/// A corpus that clears all four targets with zero errors and a passing latency run.
fn qualified_input() -> ReportInput {
    let mut trials = Vec::new();
    for index in 0..299 {
        trials.push(trial(
            &format!("far1n-{index:03}"),
            "eval-s01",
            Path::OneToN,
            GroundTruth::Impostor,
            TrialOutcome::Reject,
        ));
    }
    for index in 0..29 {
        trials.push(trial(
            &format!("gf1n-{index:03}"),
            "eval-s01",
            Path::OneToN,
            genuine("spk_a"),
            matched("spk_a"),
        ));
    }
    for index in 0..299 {
        trials.push(trial(
            &format!("far11-{index:03}"),
            "eval-s01",
            Path::OneToOne,
            GroundTruth::Impostor,
            TrialOutcome::Reject,
        ));
    }
    for index in 0..29 {
        trials.push(trial(
            &format!("frr11-{index:03}"),
            "eval-s01",
            Path::OneToOne,
            genuine("spk_a"),
            matched("spk_a"),
        ));
    }
    ReportInput {
        protocol: protocol(),
        scope: scope(),
        trials,
        latency: passing_latency(),
    }
}

fn check(report: &EvaluationReport, wanted: Check) -> &CheckReport {
    report
        .checks
        .iter()
        .find(|report| report.check == wanted)
        .expect("check present")
}

#[test]
fn fully_evidenced_corpus_qualifies() {
    let report = build_report(qualified_input()).expect("report");
    assert_eq!(report.qualification, QualificationStatus::Qualified);
    assert!(report.blocking_reasons.is_empty());
    for report_check in &report.checks {
        assert_eq!(
            report_check.status,
            CheckStatus::Pass,
            "{:?} was {:?}",
            report_check.check,
            report_check.status
        );
    }
}

#[test]
fn accounting_reconciles_attempts() {
    let mut input = qualified_input();
    input.trials.push(trial(
        "excluded-busy",
        "eval-s01",
        Path::OneToN,
        GroundTruth::Impostor,
        TrialOutcome::Excluded {
            reason: Exclusion::Busy,
        },
    ));
    input.trials.push(trial(
        "excluded-short",
        "eval-s01",
        Path::OneToN,
        genuine("spk_a"),
        TrialOutcome::Excluded {
            reason: Exclusion::Short,
        },
    ));
    let report = build_report(input).expect("report");
    let excluded: u64 = report.accounting.excluded.values().sum();
    assert_eq!(
        report.accounting.attempts,
        report.accounting.decided + excluded
    );
    assert_eq!(report.accounting.excluded[&Exclusion::Busy], 1);
    assert_eq!(report.accounting.excluded[&Exclusion::Short], 1);
    // Excluded attempts must not inflate a check denominator.
    assert_eq!(check(&report, Check::FalseAcceptOneToN).trials, 299);
}

#[test]
fn one_to_n_genuine_failure_splits_misidentification_and_rejection() {
    let mut input = qualified_input();
    input.trials.push(trial(
        "gf-misid",
        "eval-s01",
        Path::OneToN,
        genuine("spk_a"),
        matched("spk_b"),
    ));
    input.trials.push(trial(
        "gf-reject",
        "eval-s01",
        Path::OneToN,
        genuine("spk_a"),
        TrialOutcome::Reject,
    ));
    let report = build_report(input).expect("report");
    let gf = check(&report, Check::GenuineFailureOneToN);
    assert_eq!(gf.trials, 31);
    assert_eq!(gf.errors, 2);
    assert_eq!(gf.misidentification, 1);
    assert_eq!(gf.pure_rejection, 1);
    assert_eq!(report.accounting.misidentification, 1);
    assert_eq!(report.accounting.pure_rejection, 1);
}

#[test]
fn zero_error_298_trials_is_preliminary_299_passes() {
    let mut input = qualified_input();
    // Drop one impostor 1:N trial so the FAR denominator is 298.
    input
        .trials
        .retain(|trial| trial.sample_code != "far1n-298");
    let report = build_report(input).expect("report");
    let far = check(&report, Check::FalseAcceptOneToN);
    assert_eq!(far.trials, 298);
    assert_eq!(far.errors, 0);
    assert_eq!(far.status, CheckStatus::Preliminary);
    assert!(far.upper_bound.unwrap() > 0.01);
}

#[test]
fn zero_trials_is_not_run() {
    let input = ReportInput {
        protocol: protocol(),
        scope: scope(),
        trials: Vec::new(),
        latency: LatencySamples::default(),
    };
    let report = build_report(input).expect("report");
    assert_eq!(report.qualification, QualificationStatus::NotRun);
    for report_check in &report.checks {
        assert_eq!(report_check.status, CheckStatus::NotRun);
    }
}

#[test]
fn all_error_trials_fail() {
    let trials = (0..5)
        .map(|index| {
            trial(
                &format!("far-{index}"),
                "eval-s01",
                Path::OneToN,
                GroundTruth::Impostor,
                matched("spk_a"),
            )
        })
        .collect();
    let input = ReportInput {
        protocol: protocol(),
        scope: scope(),
        trials,
        latency: LatencySamples::default(),
    };
    let report = build_report(input).expect("report");
    let far = check(&report, Check::FalseAcceptOneToN);
    assert_eq!(far.errors, 5);
    assert_eq!(far.status, CheckStatus::Fail);
    assert_eq!(report.qualification, QualificationStatus::Failed);
    assert!(
        report
            .blocking_reasons
            .contains(&BlockingReason::CheckFailed(Check::FalseAcceptOneToN))
    );
}

#[test]
fn duplicate_sample_code_is_rejected() {
    let mut input = qualified_input();
    let duplicate = input.trials[0].clone();
    input.trials.push(duplicate);
    let error = build_report(input).expect_err("duplicate must fail");
    assert_eq!(
        error,
        EvaluationError::DuplicateSampleCode("far1n-000".into())
    );
}

#[test]
fn held_out_reused_for_tuning_is_preliminary() {
    let mut input = qualified_input();
    input.protocol.tuning_epoch = 5;
    let report = build_report(input).expect("report");
    assert_eq!(report.qualification, QualificationStatus::Preliminary);
    assert!(
        report
            .blocking_reasons
            .contains(&BlockingReason::HeldOutReusedForTuning)
    );
}

#[test]
fn evaluation_session_overlapping_enrollment_is_flagged() {
    let mut input = qualified_input();
    input.protocol.enrollment_sessions.insert("eval-s01".into());
    let report = build_report(input).expect("report");
    assert_eq!(report.qualification, QualificationStatus::Preliminary);
    assert!(
        report
            .blocking_reasons
            .contains(&BlockingReason::EvaluationSessionOverlapsEnrollment)
    );
}

#[test]
fn unpinned_scope_is_flagged() {
    let mut input = qualified_input();
    input.scope.threads = 0;
    let report = build_report(input).expect("report");
    assert!(
        report
            .blocking_reasons
            .contains(&BlockingReason::UnpinnedScope)
    );
}

#[test]
fn failing_latency_blocks_qualification() {
    let mut input = qualified_input();
    input.latency.inference_ms = vec![250.0; 20];
    let report = build_report(input).expect("report");
    assert_eq!(report.latency.inference_status, CheckStatus::Fail);
    assert_eq!(report.qualification, QualificationStatus::Failed);
    assert!(
        report
            .blocking_reasons
            .contains(&BlockingReason::LatencyFailed)
    );
}

#[test]
fn qualified_report_is_privacy_safe() {
    let report = build_report(qualified_input()).expect("report");
    let value = serde_json::to_value(&report).expect("json");
    assert_privacy_safe(&value).expect("report must be privacy safe");
}

#[test]
fn privacy_guard_rejects_forbidden_material() {
    let cases = [
        serde_json::json!({ "audio": [0.0, 0.1] }),
        serde_json::json!({ "embedding": [0.0, 0.1] }),
        serde_json::json!({ "note": "/var/lib/corpus/held-out.wav" }),
        serde_json::json!({ "digest": "a".repeat(64) }),
        serde_json::json!({ "transcript": "xin chao" }),
    ];
    for case in cases {
        assert!(
            assert_privacy_safe(&case).is_err(),
            "expected violation for {case}"
        );
    }
}
