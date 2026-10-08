//! Ticket 18: the pilot report built from the *production* scoring seam.
//!
//! These drive a real `SpeakerRuntime` through a real provider-runtime lease and score with the
//! production `ObservePlan`, so trial classification (genuine success, misidentification, pure
//! rejection, false accept) is exercised without native models or a network. A scripted
//! qualification extractor substitutes for CAM++.

use std::collections::VecDeque;
use std::sync::{Arc, Mutex};

use voice_agent_server::{
    audio::PcmF32Mono,
    database::DesiredProvider,
    lifecycle::AdmissionGate,
    providers::{
        RuntimeCatalog,
        speaker::{SpeakerError, SpeakerProvider},
    },
    services::provider_runtime::{
        PreparedRuntime, ProviderRuntimeManager, ResourceLease, RuntimeError, RuntimeLimits,
        RuntimeMaterializer, RuntimeResource,
    },
    session::{OBSERVE_VERIFY_THRESHOLD, ObserveCandidate, ObservePlan, SpeakerPolicyMode},
    speaker_evaluation::{
        Check, CheckStatus, EvaluationReport, GroundTruth, LatencySamples, Path, ProtocolPin,
        QualificationStatus, ReportInput, ScopePin, Trial, TrialOutcome, build_report,
    },
    workers::{ProviderRuntimeAdmission, SpeakerRuntime},
};

/// Pops one embedding per extraction, in order, so each trial is deterministic.
struct ScriptedSpeaker {
    embeddings: Arc<Mutex<VecDeque<Vec<f32>>>>,
}

impl SpeakerProvider for ScriptedSpeaker {
    fn dimension(&self) -> usize {
        3
    }

    fn extract(&mut self, _pcm: &PcmF32Mono) -> Result<Vec<f32>, SpeakerError> {
        self.embeddings
            .lock()
            .expect("script poisoned")
            .pop_front()
            .ok_or(SpeakerError::Unavailable)
    }
}

struct ScriptedResource(Arc<SpeakerRuntime>);

impl RuntimeResource for ScriptedResource {
    fn unload(&self) -> bool {
        self.0.shutdown_acknowledged()
    }

    fn runtimes_for(
        &self,
        snapshot: &DesiredProvider,
        quota: ProviderRuntimeAdmission,
    ) -> Option<RuntimeCatalog> {
        Some(RuntimeCatalog::single_speaker(
            snapshot.key.clone(),
            Arc::new(self.0.logical_view(quota)),
        ))
    }
}

struct ScriptedFactory {
    embeddings: Arc<Mutex<VecDeque<Vec<f32>>>>,
}

impl RuntimeMaterializer for ScriptedFactory {
    fn estimated_peak_bytes(&self, _: &DesiredProvider) -> Result<u64, RuntimeError> {
        Ok(1)
    }

    fn logical_capacity(&self, _: &DesiredProvider) -> Result<usize, RuntimeError> {
        Ok(1)
    }

    fn build(
        &self,
        _: &DesiredProvider,
        _: Option<PreparedRuntime>,
        quota: ProviderRuntimeAdmission,
    ) -> Result<Arc<dyn RuntimeResource>, RuntimeError> {
        let provider = ScriptedSpeaker {
            embeddings: self.embeddings.clone(),
        };
        let runtime = SpeakerRuntime::new(Box::new(provider), quota).unwrap();
        Ok(Arc::new(ScriptedResource(Arc::new(runtime))))
    }
}

async fn runtime_with(embeddings: Vec<[f32; 3]>) -> (Arc<SpeakerRuntime>, ResourceLease) {
    // `SpeakerRuntime::new` consumes one warmup extraction before the queue is exposed.
    let mut all = vec![[1.0, 0.0, 0.0]];
    all.extend(embeddings);
    let queue = Arc::new(Mutex::new(
        all.into_iter().map(Vec::from).collect::<VecDeque<_>>(),
    ));
    let manager = ProviderRuntimeManager::new(
        RuntimeLimits {
            max_parallel_loads: 1,
            max_pending_loads: 1,
            max_waiters: 4,
            max_resident_bytes: 8192,
            max_resources: 2,
            max_version_entries: 8,
            admission_timeout_ms: 1000,
            failure_cooldown_ms: 10,
            idle_ttl_ms: 60_000,
        },
        Arc::new(ScriptedFactory { embeddings: queue }),
        AdmissionGate::open(),
    )
    .unwrap();
    let lease = manager
        .acquire(DesiredProvider {
            id: 1,
            key: "speaker".into(),
            kind: "speaker".into(),
            adapter: "campplus".into(),
            revision: 1,
            config_json: "{}".into(),
            secret_ref: None,
        })
        .await
        .unwrap();
    let runtime = lease.runtimes().unwrap().speaker("speaker").unwrap();
    (runtime, lease)
}

fn plan(runtime: &SpeakerRuntime, candidates: &[(&str, [f32; 3])]) -> ObservePlan {
    ObservePlan {
        agent_id: 7,
        template_id: 3,
        embedding_space: runtime.embedding_space_id().to_owned(),
        catalog_revision: 42,
        policy: SpeakerPolicyMode::Observe,
        candidates: candidates
            .iter()
            .enumerate()
            .map(|(index, (key, vector))| ObserveCandidate {
                speaker_id: index as i64 + 1,
                key: (*key).into(),
                vector: vector.to_vec(),
            })
            .collect(),
    }
}

/// Score one embedding through the production plan, exactly as Observe does.
async fn decide(
    runtime: &SpeakerRuntime,
    lease: &ResourceLease,
    plan: &ObservePlan,
    embedding: [f32; 3],
) -> TrialOutcome {
    let pcm = PcmF32Mono::new(vec![0.1; 16_000], 16_000);
    let extracted = runtime.extract(pcm, lease.clone()).await.unwrap();
    assert_eq!(
        extracted, embedding,
        "runtime returned the scripted embedding"
    );
    let best = plan
        .score(&extracted)
        .into_iter()
        .next()
        .expect("a candidate");
    if best.score >= OBSERVE_VERIFY_THRESHOLD {
        TrialOutcome::Match { identity: best.key }
    } else {
        TrialOutcome::Reject
    }
}

fn trial(sample: &str, path: Path, ground_truth: GroundTruth, outcome: TrialOutcome) -> Trial {
    Trial {
        sample_code: sample.into(),
        session_code: "eval-s01".into(),
        path,
        ground_truth,
        outcome,
    }
}

fn pin() -> (ProtocolPin, ScopePin) {
    (
        ProtocolPin {
            corpus_version: "corpus-v1".into(),
            protocol_version: "pilot-v1".into(),
            embedding_space: "speaker:campplus".into(),
            voiceprint_revision: 1,
            catalog_revision: 42,
            candidate_set_id: "set-001".into(),
            holdout_epoch: 1,
            tuning_epoch: 1,
            enrollment_sessions: ["enroll-s01".to_string()].into_iter().collect(),
        },
        ScopePin {
            machine: "pilot-host-1".into(),
            model_revision: "campplus-v1".into(),
            threads: 1,
            worker: "speaker-0".into(),
            queue_depth: 1,
            workload: "pilot-1n".into(),
            audio_conditions: "quiet-office".into(),
        },
    )
}

#[tokio::test]
async fn production_scoring_classifies_trials_into_the_report() {
    let alice = [1.0, 0.0, 0.0];
    let bob = [0.0, 1.0, 0.0];
    // Order matches the trials below.
    let (runtime, lease) = runtime_with(vec![
        alice,           // genuine alice -> match alice
        bob,             // genuine alice, wrong embedding -> match bob (misidentification)
        [0.0, 0.0, 1.0], // genuine alice, orthogonal -> reject (pure rejection)
        [0.0, 0.0, 1.0], // impostor -> reject (true rejection)
        bob,             // impostor -> match bob (false accept)
    ])
    .await;
    let plan = plan(&runtime, &[("alice", alice), ("bob", bob)]);

    let outcomes = [
        decide(&runtime, &lease, &plan, alice).await,
        decide(&runtime, &lease, &plan, bob).await,
        decide(&runtime, &lease, &plan, [0.0, 0.0, 1.0]).await,
        decide(&runtime, &lease, &plan, [0.0, 0.0, 1.0]).await,
        decide(&runtime, &lease, &plan, bob).await,
    ];

    let trials = vec![
        trial(
            "t1",
            Path::OneToN,
            GroundTruth::Genuine {
                identity: "alice".into(),
            },
            outcomes[0].clone(),
        ),
        trial(
            "t2",
            Path::OneToN,
            GroundTruth::Genuine {
                identity: "alice".into(),
            },
            outcomes[1].clone(),
        ),
        trial(
            "t3",
            Path::OneToN,
            GroundTruth::Genuine {
                identity: "alice".into(),
            },
            outcomes[2].clone(),
        ),
        trial(
            "t4",
            Path::OneToN,
            GroundTruth::Impostor,
            outcomes[3].clone(),
        ),
        trial(
            "t5",
            Path::OneToN,
            GroundTruth::Impostor,
            outcomes[4].clone(),
        ),
    ];
    let (protocol, scope) = pin();
    let report: EvaluationReport = build_report(ReportInput {
        protocol,
        scope,
        trials,
        latency: LatencySamples::default(),
    })
    .expect("report");

    let gf = report
        .checks
        .iter()
        .find(|check| check.check == Check::GenuineFailureOneToN)
        .unwrap();
    assert_eq!(gf.trials, 3);
    assert_eq!(gf.errors, 2);
    assert_eq!(gf.misidentification, 1);
    assert_eq!(gf.pure_rejection, 1);

    let far = report
        .checks
        .iter()
        .find(|check| check.check == Check::FalseAcceptOneToN)
        .unwrap();
    assert_eq!(far.trials, 2);
    assert_eq!(far.errors, 1);

    // Too few trials to prove anything, but the point estimates (2/3 genuine failures, 1/2 false
    // accepts) blow past their targets, so the report must fail — never `Qualified`.
    assert_eq!(gf.status, CheckStatus::Fail);
    assert_eq!(far.status, CheckStatus::Fail);
    assert_ne!(report.qualification, QualificationStatus::Qualified);
    assert_eq!(report.qualification, QualificationStatus::Failed);

    let value = serde_json::to_value(&report).unwrap();
    voice_agent_server::speaker_evaluation::assert_privacy_safe(&value).unwrap();
}
