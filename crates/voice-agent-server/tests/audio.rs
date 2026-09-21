use voice_agent_server::audio::{
    AudioFrameDropReason, CaptureOutcome, DecodeOutcome, DownlinkOpusEncoder, DownlinkPcmFrame,
    ManualCapture, Pcm16Mono, UplinkOpusDecoder, UplinkPcmFrame,
};

fn uplink_frame(value: i16) -> UplinkPcmFrame {
    UplinkPcmFrame::try_new(Pcm16Mono::new(vec![value; 960])).unwrap()
}

#[test]
fn opus_round_trip_returns_a_canonical_uplink_frame() {
    let frame = DownlinkPcmFrame::try_new(Pcm16Mono::new(
        (0..1_440)
            .map(|sample| (sample as i16).wrapping_mul(19))
            .collect(),
    ))
    .unwrap();
    let packet = DownlinkOpusEncoder::new(65_536)
        .unwrap()
        .encode(frame)
        .unwrap();

    let mut decoder = UplinkOpusDecoder::new().unwrap();
    assert!(matches!(
        decoder.decode(packet.as_bytes()),
        DecodeOutcome::Frame(_)
    ));
}

#[test]
fn downlink_encoder_rejects_packets_larger_than_the_transport_cap() {
    let frame = DownlinkPcmFrame::try_new(Pcm16Mono::new(vec![1; 1_440])).unwrap();
    let error = DownlinkOpusEncoder::new(1)
        .unwrap()
        .encode(frame)
        .unwrap_err();
    assert!(matches!(
        error,
        voice_agent_server::audio::AudioError::EncodedPacketTooLarge { max: 1, .. }
    ));
}

#[test]
fn decoder_drops_empty_and_oversized_packets_without_panicking() {
    let mut decoder = UplinkOpusDecoder::new().unwrap();
    assert_eq!(
        decoder.decode(&[]),
        DecodeOutcome::Dropped(AudioFrameDropReason::EmptyPacket)
    );
    assert_eq!(
        decoder.decode(&vec![0; 4_001]),
        DecodeOutcome::Dropped(AudioFrameDropReason::PacketTooLarge)
    );
}

#[test]
fn manual_capture_preserves_frames_in_order() {
    let mut capture = ManualCapture::new(2).unwrap();
    capture.start();
    capture.push(uplink_frame(1));
    capture.push(uplink_frame(2));

    let CaptureOutcome::Utterance(utterance) = capture.stop() else {
        panic!("expected an uplink audio utterance");
    };
    assert_eq!(utterance.samples().len(), 1_920);
    assert_eq!(&utterance.samples()[..960], vec![1; 960].as_slice());
    assert_eq!(&utterance.samples()[960..], vec![2; 960].as_slice());
}

#[test]
fn manual_capture_overflow_discards_the_whole_utterance() {
    let mut capture = ManualCapture::new(1).unwrap();
    capture.start();
    capture.push(uplink_frame(7));
    capture.push(uplink_frame(8));

    assert_eq!(capture.stop(), CaptureOutcome::Overflowed);
}

#[test]
fn manual_capture_restart_and_abort_discard_collected_pcm() {
    let mut capture = ManualCapture::new(2).unwrap();
    capture.start();
    capture.push(uplink_frame(3));
    capture.restart();
    assert_eq!(capture.stop(), CaptureOutcome::Empty);

    capture.start();
    capture.push(uplink_frame(4));
    capture.abort();
    assert_eq!(capture.stop(), CaptureOutcome::Empty);
}

#[test]
fn direction_specific_pcm_frames_reject_the_wrong_shape() {
    assert!(UplinkPcmFrame::try_new(Pcm16Mono::new(vec![0; 1_440])).is_err());
    assert!(DownlinkPcmFrame::try_new(Pcm16Mono::new(vec![0; 960])).is_err());
}
