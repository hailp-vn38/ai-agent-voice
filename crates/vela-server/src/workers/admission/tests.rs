use super::*;

#[test]
fn single_worker_allows_one_diagnostic_when_idle() {
    let gate = ProviderRuntimeAdmission::new(1, 1);
    let diagnostic = gate.try_admit(ProviderWorkloadClass::Diagnostic).unwrap();
    assert!(gate.try_admit(ProviderWorkloadClass::Diagnostic).is_err());
    assert!(gate.try_admit(ProviderWorkloadClass::Voice).is_err());
    drop(diagnostic);
    let voice = gate.try_admit(ProviderWorkloadClass::Voice).unwrap();
    assert!(gate.try_admit(ProviderWorkloadClass::Diagnostic).is_err());
    drop(voice);
    assert!(gate.try_admit(ProviderWorkloadClass::Diagnostic).is_ok());
}

#[test]
fn diagnostic_never_consumes_the_voice_reservation() {
    let gate = ProviderRuntimeAdmission::new(2, 1);
    let _diagnostic = gate.try_admit(ProviderWorkloadClass::Diagnostic).unwrap();
    assert!(matches!(
        gate.try_admit(ProviderWorkloadClass::Diagnostic),
        Err(ProviderAdmissionError::Capacity)
    ));
    assert!(gate.try_admit(ProviderWorkloadClass::Voice).is_ok());
}

#[test]
fn diagnostic_cannot_take_the_last_slot_after_voice_started() {
    let gate = ProviderRuntimeAdmission::new(2, 1);
    let _voice = gate.try_admit(ProviderWorkloadClass::Voice).unwrap();
    assert!(matches!(
        gate.try_admit(ProviderWorkloadClass::Diagnostic),
        Err(ProviderAdmissionError::Capacity)
    ));
}

#[test]
fn logical_views_have_independent_quotas_and_one_physical_limit() {
    let physical = ProviderRuntimeAdmission::new(3, 1);
    let first = ProviderRuntimeAdmission::new(2, 1).composed(&physical);
    let second = ProviderRuntimeAdmission::new(2, 1).composed(&physical);
    let a = first.try_admit(ProviderWorkloadClass::Voice).unwrap();
    let b = first.try_admit(ProviderWorkloadClass::Voice).unwrap();
    assert!(first.try_admit(ProviderWorkloadClass::Voice).is_err());
    let c = second.try_admit(ProviderWorkloadClass::Voice).unwrap();
    assert!(second.try_admit(ProviderWorkloadClass::Voice).is_err());
    drop(a);
    let d = second.try_admit(ProviderWorkloadClass::Voice).unwrap();
    assert!(second.try_admit(ProviderWorkloadClass::Voice).is_err());
    drop((b, c, d));
    assert_eq!(physical.active_work(), 0);
}
