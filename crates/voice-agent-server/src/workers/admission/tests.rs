use super::*;

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
