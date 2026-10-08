//! Bounded-pilot speaker evaluation: trial accounting, exact one-sided bounds and a
//! privacy-safe PASS/FAIL/NOT_RUN report.
//!
//! Ticket 18 / guide §11. The pilot proves four error-rate targets against one held-out corpus:
//!
//! * 1:N false accept (an unregistered voice accepted as a candidate) — `<= 1%`
//! * 1:N genuine failure (a registered voice rejected or matched to the wrong identity) — `<= 10%`
//! * 1:1 false accept (a different voice accepted as the locked identity) — `<= 1%`
//! * 1:1 false reject (the locked identity rejected) — `<= 10%`
//!
//! This module is deliberately pure: the operator harness runs the corpus through the production
//! runtime and hands the classified [`Trial`]s to [`build_report`]. Missing or contaminated
//! evidence yields `NOT_RUN`/`PRELIMINARY`, never a fabricated qualification.

pub mod binomial;

use std::collections::{BTreeMap, BTreeSet};

use serde::{Deserialize, Serialize};

use crate::benchmark::{MetricSummary, summarize};

pub use binomial::{min_zero_error_trials, upper_bound};

/// Which identity check a trial belongs to.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Path {
    OneToN,
    OneToOne,
}

/// The four independent checks the report proves.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Check {
    FalseAcceptOneToN,
    GenuineFailureOneToN,
    FalseAcceptOneToOne,
    FalseRejectOneToOne,
}

impl Check {
    /// All four checks, in report order.
    pub const ALL: [Check; 4] = [
        Check::FalseAcceptOneToN,
        Check::GenuineFailureOneToN,
        Check::FalseAcceptOneToOne,
        Check::FalseRejectOneToOne,
    ];

    pub fn path(self) -> Path {
        match self {
            Check::FalseAcceptOneToN | Check::GenuineFailureOneToN => Path::OneToN,
            Check::FalseAcceptOneToOne | Check::FalseRejectOneToOne => Path::OneToOne,
        }
    }

    /// One-sided upper-bound target. No simultaneous 95% claim is made across checks.
    pub fn target(self) -> f64 {
        match self {
            Check::FalseAcceptOneToN | Check::FalseAcceptOneToOne => 0.01,
            Check::GenuineFailureOneToN | Check::FalseRejectOneToOne => 0.10,
        }
    }
}

/// Why an attempt produced no usable decision. Reported, never silently dropped.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Exclusion {
    /// Utterance shorter than the enrollment profile accepts.
    Short,
    /// Utterance present but too noisy/silent to embed.
    Quality,
    /// Runtime was busy and the attempt was dropped by policy.
    Busy,
    /// Inference or acquisition timed out.
    Timeout,
    /// The native/runtime path failed.
    RuntimeFailure,
    /// The recording was replayed from a prior trial.
    Replay,
    /// The recording overlaps another enrollment/session recording.
    Overlap,
}

/// Ground truth for one attempt.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum GroundTruth {
    /// The true speaker. For 1:N `identity` must be in the candidate set; for 1:1 it is locked.
    Genuine { identity: String },
    /// A voice that is not the genuine identity.
    Impostor,
}

/// What the runtime decided for one attempt.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TrialOutcome {
    Match { identity: String },
    Reject,
    Excluded { reason: Exclusion },
}

/// One utterance, one attempt. `sample_code` is a coded identifier, never a path or real name.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Trial {
    pub sample_code: String,
    /// Recording session, used to keep the held-out set disjoint from enrollment.
    pub session_code: String,
    pub path: Path,
    pub ground_truth: GroundTruth,
    pub outcome: TrialOutcome,
}

/// Immutable protocol the report is pinned to. Changing any field invalidates the evidence.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProtocolPin {
    pub corpus_version: String,
    pub protocol_version: String,
    pub embedding_space: String,
    pub voiceprint_revision: i64,
    pub catalog_revision: i64,
    /// Opaque, non-secret candidate-set identifier (no digest of held-out material).
    pub candidate_set_id: String,
    /// Epoch the held-out corpus was frozen in, and the tuning epoch that last touched the model.
    pub holdout_epoch: u32,
    pub tuning_epoch: u32,
    /// Sessions used to enroll the candidate set. Held-out sessions must not intersect.
    pub enrollment_sessions: BTreeSet<String>,
}

/// Machine/worker conditions the latency numbers are valid under.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ScopePin {
    pub machine: String,
    pub model_revision: String,
    pub threads: u32,
    pub worker: String,
    pub queue_depth: u32,
    pub workload: String,
    pub audio_conditions: String,
}

impl ScopePin {
    fn is_pinned(&self) -> bool {
        !self.machine.is_empty()
            && !self.model_revision.is_empty()
            && self.threads > 0
            && !self.worker.is_empty()
            && !self.workload.is_empty()
            && !self.audio_conditions.is_empty()
    }
}

/// Raw latency samples (milliseconds). Empty means "not run".
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct LatencySamples {
    /// Warm inference time per 4-second window.
    pub inference_ms: Vec<f64>,
    /// Terminal-boundary to gate-decision wait.
    pub gate_wait_ms: Vec<f64>,
    /// Time spent queued behind other work.
    pub queue_ms: Vec<f64>,
    /// ASR time, reported separately from the gate.
    pub asr_ms: Vec<f64>,
    /// Barrier/barrier-equivalent wait, reported separately.
    pub barrier_ms: Vec<f64>,
}

/// Per-check PASS/FAIL/PRELIMINARY/NOT_RUN.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum CheckStatus {
    Pass,
    Fail,
    Preliminary,
    NotRun,
}

/// One check's evidence.
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct CheckReport {
    pub check: Check,
    pub path: Path,
    pub target: f64,
    pub trials: u64,
    pub errors: u64,
    pub upper_bound: Option<f64>,
    pub status: CheckStatus,
    /// 1:N genuine failures split into the two distinct outcomes; zero on other checks.
    pub misidentification: u64,
    pub pure_rejection: u64,
}

/// Latency evidence. Targets: inference p95 `<= 200ms`, gate wait p95 `<= 500ms`.
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct LatencyReport {
    pub inference_target_ms: f64,
    pub gate_wait_target_ms: f64,
    pub inference: Option<MetricSummary>,
    pub gate_wait: Option<MetricSummary>,
    pub queue: Option<MetricSummary>,
    pub asr: Option<MetricSummary>,
    pub barrier: Option<MetricSummary>,
    pub inference_status: CheckStatus,
    pub gate_wait_status: CheckStatus,
}

/// Trial totals, reconciled so excluded attempts are never lost.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct TrialAccounting {
    pub attempts: u64,
    pub decided: u64,
    pub excluded: BTreeMap<Exclusion, u64>,
    pub genuine_success: u64,
    pub misidentification: u64,
    pub pure_rejection: u64,
    pub false_accept: u64,
    pub true_rejection: u64,
}

/// Overall qualification state. `Qualified` is only produced from complete, uncontaminated
/// evidence that meets every target.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum QualificationStatus {
    Qualified,
    Preliminary,
    Failed,
    NotRun,
}

/// Why the report is not `Qualified`.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum BlockingReason {
    CheckFailed(Check),
    InsufficientEvidence(Check),
    LatencyFailed,
    LatencyNotRun,
    HeldOutReusedForTuning,
    EvaluationSessionOverlapsEnrollment,
    UnpinnedScope,
}

/// The pilot report. Contains coded identifiers only — see [`assert_privacy_safe`].
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct EvaluationReport {
    pub protocol: ProtocolPin,
    pub scope: ScopePin,
    pub accounting: TrialAccounting,
    pub checks: Vec<CheckReport>,
    pub latency: LatencyReport,
    pub qualification: QualificationStatus,
    pub blocking_reasons: Vec<BlockingReason>,
}

/// Inputs the operator harness collected.
#[derive(Clone, Debug)]
pub struct ReportInput {
    pub protocol: ProtocolPin,
    pub scope: ScopePin,
    pub trials: Vec<Trial>,
    pub latency: LatencySamples,
}

/// A report cannot be produced from a corpus that reuses a sample across trials — that is the
/// "multiply trials by replaying clips" failure the protocol forbids.
#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error)]
pub enum EvaluationError {
    #[error("sample code {0:?} appears in more than one trial")]
    DuplicateSampleCode(String),
}

/// Build the privacy-safe report from collected trials.
pub fn build_report(input: ReportInput) -> Result<EvaluationReport, EvaluationError> {
    let ReportInput {
        protocol,
        scope,
        trials,
        latency,
    } = input;

    let mut seen = BTreeSet::new();
    for trial in &trials {
        if !seen.insert(trial.sample_code.as_str()) {
            return Err(EvaluationError::DuplicateSampleCode(
                trial.sample_code.clone(),
            ));
        }
    }

    let accounting = account(&trials);
    let checks = Check::ALL
        .iter()
        .copied()
        .map(|check| check_report(check, &trials))
        .collect::<Vec<_>>();
    let latency = latency_report(&latency);

    let mut blocking_reasons = Vec::new();
    let mut any_failed = false;
    for report in &checks {
        match report.status {
            CheckStatus::Fail => {
                any_failed = true;
                blocking_reasons.push(BlockingReason::CheckFailed(report.check));
            }
            CheckStatus::Preliminary | CheckStatus::NotRun => {
                blocking_reasons.push(BlockingReason::InsufficientEvidence(report.check));
            }
            CheckStatus::Pass => {}
        }
    }
    match latency_status(&latency) {
        CheckStatus::Fail => {
            any_failed = true;
            blocking_reasons.push(BlockingReason::LatencyFailed);
        }
        CheckStatus::Preliminary | CheckStatus::NotRun => {
            blocking_reasons.push(BlockingReason::LatencyNotRun);
        }
        CheckStatus::Pass => {}
    }
    if protocol.tuning_epoch > protocol.holdout_epoch {
        blocking_reasons.push(BlockingReason::HeldOutReusedForTuning);
    }
    let evaluation_sessions = trials
        .iter()
        .map(|trial| trial.session_code.as_str())
        .collect::<BTreeSet<_>>();
    if evaluation_sessions
        .iter()
        .any(|session| protocol.enrollment_sessions.contains(*session))
    {
        blocking_reasons.push(BlockingReason::EvaluationSessionOverlapsEnrollment);
    }
    if !scope.is_pinned() {
        blocking_reasons.push(BlockingReason::UnpinnedScope);
    }

    let no_evidence =
        accounting.decided == 0 && latency.inference.is_none() && latency.gate_wait.is_none();
    let qualification = if any_failed {
        QualificationStatus::Failed
    } else if blocking_reasons.is_empty() {
        QualificationStatus::Qualified
    } else if no_evidence {
        QualificationStatus::NotRun
    } else {
        QualificationStatus::Preliminary
    };

    Ok(EvaluationReport {
        protocol,
        scope,
        accounting,
        checks,
        latency,
        qualification,
        blocking_reasons,
    })
}

fn account(trials: &[Trial]) -> TrialAccounting {
    let mut accounting = TrialAccounting {
        attempts: trials.len() as u64,
        decided: 0,
        excluded: BTreeMap::new(),
        genuine_success: 0,
        misidentification: 0,
        pure_rejection: 0,
        false_accept: 0,
        true_rejection: 0,
    };
    for trial in trials {
        match (&trial.ground_truth, &trial.outcome) {
            (_, TrialOutcome::Excluded { reason }) => {
                *accounting.excluded.entry(*reason).or_insert(0) += 1;
            }
            (GroundTruth::Genuine { identity }, TrialOutcome::Match { identity: matched }) => {
                accounting.decided += 1;
                if identity == matched {
                    accounting.genuine_success += 1;
                } else {
                    accounting.misidentification += 1;
                }
            }
            (GroundTruth::Genuine { .. }, TrialOutcome::Reject) => {
                accounting.decided += 1;
                accounting.pure_rejection += 1;
            }
            (GroundTruth::Impostor, TrialOutcome::Match { .. }) => {
                accounting.decided += 1;
                accounting.false_accept += 1;
            }
            (GroundTruth::Impostor, TrialOutcome::Reject) => {
                accounting.decided += 1;
                accounting.true_rejection += 1;
            }
        }
    }
    accounting
}

fn check_report(check: Check, trials: &[Trial]) -> CheckReport {
    let mut trials_n = 0u64;
    let mut errors = 0u64;
    let mut misidentification = 0u64;
    let mut pure_rejection = 0u64;

    for trial in trials.iter().filter(|trial| trial.path == check.path()) {
        let decision = match &trial.outcome {
            TrialOutcome::Match { identity } => Some(identity.as_str()),
            TrialOutcome::Reject => None,
            TrialOutcome::Excluded { .. } => continue,
        };
        let impostor = matches!(trial.ground_truth, GroundTruth::Impostor);
        match check {
            // Impostor accepted as some candidate.
            Check::FalseAcceptOneToN | Check::FalseAcceptOneToOne => {
                if impostor {
                    trials_n += 1;
                    if decision.is_some() {
                        errors += 1;
                    }
                }
            }
            // Genuine rejected, or matched to the wrong identity.
            Check::GenuineFailureOneToN | Check::FalseRejectOneToOne => {
                let GroundTruth::Genuine { identity: truth } = &trial.ground_truth else {
                    continue;
                };
                trials_n += 1;
                match decision {
                    Some(matched) if matched == truth => {}
                    Some(_) => {
                        errors += 1;
                        misidentification += 1;
                    }
                    None => {
                        errors += 1;
                        pure_rejection += 1;
                    }
                }
            }
        }
    }

    let bound = upper_bound(trials_n, errors);
    let target = check.target();
    let status = if trials_n == 0 {
        CheckStatus::NotRun
    } else if errors as f64 / trials_n as f64 > target {
        CheckStatus::Fail
    } else if bound.is_some_and(|bound| bound <= target) {
        CheckStatus::Pass
    } else {
        CheckStatus::Preliminary
    };

    CheckReport {
        check,
        path: check.path(),
        target,
        trials: trials_n,
        errors,
        upper_bound: bound,
        status,
        misidentification,
        pure_rejection,
    }
}

fn latency_report(samples: &LatencySamples) -> LatencyReport {
    let inference = summarize_opt(&samples.inference_ms);
    let gate_wait = summarize_opt(&samples.gate_wait_ms);
    LatencyReport {
        inference_target_ms: 200.0,
        gate_wait_target_ms: 500.0,
        inference_status: latency_check(inference.as_ref(), 200.0),
        gate_wait_status: latency_check(gate_wait.as_ref(), 500.0),
        inference,
        gate_wait,
        queue: summarize_opt(&samples.queue_ms),
        asr: summarize_opt(&samples.asr_ms),
        barrier: summarize_opt(&samples.barrier_ms),
    }
}

fn summarize_opt(values: &[f64]) -> Option<MetricSummary> {
    (!values.is_empty()).then(|| summarize(values.iter().copied()))
}

fn latency_check(summary: Option<&MetricSummary>, target_ms: f64) -> CheckStatus {
    match summary {
        None => CheckStatus::NotRun,
        Some(summary) if summary.p95 <= target_ms => CheckStatus::Pass,
        Some(_) => CheckStatus::Fail,
    }
}

fn latency_status(report: &LatencyReport) -> CheckStatus {
    let statuses = [report.inference_status, report.gate_wait_status];
    if statuses.contains(&CheckStatus::Fail) {
        CheckStatus::Fail
    } else if statuses.iter().all(|status| *status == CheckStatus::Pass) {
        CheckStatus::Pass
    } else if statuses.iter().all(|status| *status == CheckStatus::NotRun) {
        CheckStatus::NotRun
    } else {
        CheckStatus::Preliminary
    }
}

/// Belt-and-braces guard before a report leaves the process: the report must carry no raw audio,
/// embedding, transcript, credential, path or held-out digest. Callers pass
/// `serde_json::to_value(&report)`.
pub fn assert_privacy_safe(value: &serde_json::Value) -> Result<(), PrivacyViolation> {
    let mut pointer = String::new();
    check_value(value, &mut pointer)
}

/// A forbidden key or value found in a report.
#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error)]
#[error("privacy violation at {pointer}: {reason}")]
pub struct PrivacyViolation {
    pub pointer: String,
    pub reason: String,
}

const FORBIDDEN_KEYS: &[&str] = &[
    "audio",
    "pcm",
    "wav",
    "raw",
    "samples",
    "embedding",
    "vector",
    "transcript",
    "credential",
    "token",
    "secret",
    "password",
    "filepath",
    "url",
    "digest",
    "hash",
    "sha",
    "phone",
    "email",
    "name",
    "ssn",
    "national_id",
];

fn check_value(value: &serde_json::Value, pointer: &mut String) -> Result<(), PrivacyViolation> {
    match value {
        serde_json::Value::Object(map) => {
            for (key, child) in map {
                let normalized = key.to_ascii_lowercase();
                if FORBIDDEN_KEYS.contains(&normalized.as_str()) {
                    return Err(PrivacyViolation {
                        pointer: format!("{pointer}/{key}"),
                        reason: format!("forbidden field {key:?}"),
                    });
                }
                let mut child_pointer = format!("{pointer}/{key}");
                check_value(child, &mut child_pointer)?;
            }
        }
        serde_json::Value::Array(items) => {
            for (index, child) in items.iter().enumerate() {
                let mut child_pointer = format!("{pointer}/{index}");
                check_value(child, &mut child_pointer)?;
            }
        }
        serde_json::Value::String(text) => {
            if let Some(reason) = forbidden_value(text) {
                return Err(PrivacyViolation {
                    pointer: pointer.clone(),
                    reason,
                });
            }
        }
        _ => {}
    }
    Ok(())
}

fn forbidden_value(text: &str) -> Option<String> {
    let lower = text.to_ascii_lowercase();
    if text.starts_with('/') || text.starts_with('~') {
        return Some("absolute path".into());
    }
    for extension in [".wav", ".pcm", ".flac", ".npy", ".onnx", ".opus"] {
        if lower.ends_with(extension) {
            return Some(format!("file-like value ({extension})"));
        }
    }
    // Hex digests and base64 blobs of embedding size.
    if text.len() >= 64 && text.bytes().all(|byte| byte.is_ascii_hexdigit()) {
        return Some("hex digest".into());
    }
    if text.len() >= 80
        && text.bytes().all(|byte| {
            byte.is_ascii_alphanumeric() || byte == b'=' || byte == b'/' || byte == b'+'
        })
    {
        return Some("base64 blob".into());
    }
    None
}

#[cfg(test)]
mod tests;
