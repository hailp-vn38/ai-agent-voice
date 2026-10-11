use super::*;
use std::{
    sync::{
        atomic::{AtomicBool, Ordering},
        mpsc,
    },
    time::Duration,
};

#[test]
fn local_load_overlaps_text_and_prefix_load() {
    let (started_tx, started_rx) = mpsc::sync_channel(1);
    let (release_tx, release_rx) = mpsc::sync_channel(1);
    let result = load_ar_graphs(
        move || {
            started_tx.send(()).unwrap();
            release_rx
                .recv_timeout(Duration::from_secs(1))
                .map_err(|_| TtsError::Failed)?;
            Ok((1, 2))
        },
        || {
            started_rx.recv_timeout(Duration::from_secs(1)).unwrap();
            release_tx.send(()).unwrap();
            Ok(3)
        },
    );
    assert_eq!(result.unwrap(), (1, 2, 3));
}

#[test]
fn local_failure_waits_for_the_other_load_to_finish() {
    let (started_tx, started_rx) = mpsc::sync_channel(1);
    let (failed_tx, failed_rx) = mpsc::sync_channel(1);
    let (release_tx, release_rx) = mpsc::sync_channel(1);
    let (result_tx, result_rx) = mpsc::sync_channel(1);
    let attempt = std::thread::spawn(move || {
        let result = load_ar_graphs(
            move || {
                started_tx.send(()).unwrap();
                release_rx.recv_timeout(Duration::from_secs(5)).unwrap();
                Ok((1, 2))
            },
            || {
                failed_tx.send(()).unwrap();
                Err(TtsError::Failed)
            },
        );
        result_tx.send(result).unwrap();
    });
    started_rx.recv_timeout(Duration::from_secs(1)).unwrap();
    let failure_started = failed_rx.recv_timeout(Duration::from_secs(1));
    let returned_early = result_rx.recv_timeout(Duration::from_millis(100));
    release_tx.send(()).unwrap();
    attempt.join().unwrap();
    assert!(failure_started.is_ok());
    assert!(matches!(
        returned_early,
        Err(mpsc::RecvTimeoutError::Timeout)
    ));
    assert!(matches!(result_rx.recv().unwrap(), Err(TtsError::Failed)));
}

#[test]
fn a_text_failure_still_finishes_the_local_load() {
    let finished = AtomicBool::new(false);
    let result = load_ar_graphs(
        || Err::<(usize, usize), _>(TtsError::UnsupportedVoice("failed".into())),
        || {
            finished.store(true, Ordering::Release);
            Ok(3)
        },
    );
    assert!(finished.load(Ordering::Acquire));
    assert!(matches!(result, Err(TtsError::UnsupportedVoice(_))));
}

#[test]
fn a_loader_panic_is_an_error_after_the_other_load_finishes() {
    let finished = AtomicBool::new(false);
    let result = load_ar_graphs(
        || -> Result<(usize, usize), TtsError> { panic!("native loader panic") },
        || {
            finished.store(true, Ordering::Release);
            Ok(3)
        },
    );
    assert!(finished.load(Ordering::Acquire));
    assert!(matches!(result, Err(TtsError::Failed)));
}
