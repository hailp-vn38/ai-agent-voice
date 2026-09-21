use super::{
    AudioError, DownlinkPcmFrame, Pcm16Mono, UplinkPcmFrame, DOWNLINK_FRAME_SAMPLES,
    UPLINK_FRAME_SAMPLES,
};
use opus2::{Application, Bitrate, Channels, Decoder, Encoder};

pub const MAX_UPLINK_OPUS_PACKET_BYTES: usize = 4_000;
pub const DOWNLINK_ENCODE_BUFFER_BYTES: usize = 4_000;
const UPLINK_MAX_DECODE_SAMPLES: usize = 1_920;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct OpusPacket(Vec<u8>);

impl OpusPacket {
    pub fn as_bytes(&self) -> &[u8] {
        &self.0
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AudioFrameDropReason {
    EmptyPacket,
    PacketTooLarge,
    DecodeError,
    InvalidSampleCount,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum DecodeOutcome {
    Frame(UplinkPcmFrame),
    Dropped(AudioFrameDropReason),
}

pub struct UplinkOpusDecoder {
    inner: Decoder,
    scratch: [i16; UPLINK_MAX_DECODE_SAMPLES],
}

impl UplinkOpusDecoder {
    pub fn new() -> Result<Self, AudioError> {
        Ok(Self {
            inner: Decoder::new(16_000, Channels::Mono).map_err(|_| AudioError::DecoderInit)?,
            scratch: [0; UPLINK_MAX_DECODE_SAMPLES],
        })
    }

    pub fn decode(&mut self, packet: &[u8]) -> DecodeOutcome {
        if packet.is_empty() {
            return DecodeOutcome::Dropped(AudioFrameDropReason::EmptyPacket);
        }
        if packet.len() > MAX_UPLINK_OPUS_PACKET_BYTES {
            return DecodeOutcome::Dropped(AudioFrameDropReason::PacketTooLarge);
        }
        let samples = match self.inner.decode(packet, &mut self.scratch, false) {
            Ok(samples) => samples,
            Err(_) => return DecodeOutcome::Dropped(AudioFrameDropReason::DecodeError),
        };
        if samples != UPLINK_FRAME_SAMPLES {
            return DecodeOutcome::Dropped(AudioFrameDropReason::InvalidSampleCount);
        }
        UplinkPcmFrame::try_new(Pcm16Mono::new(self.scratch[..samples].to_vec()))
            .map(DecodeOutcome::Frame)
            .unwrap_or(DecodeOutcome::Dropped(
                AudioFrameDropReason::InvalidSampleCount,
            ))
    }
}

pub struct DownlinkOpusEncoder {
    inner: Encoder,
    scratch: [u8; DOWNLINK_ENCODE_BUFFER_BYTES],
    max_packet_bytes: usize,
}

impl DownlinkOpusEncoder {
    pub fn new(max_packet_bytes: usize) -> Result<Self, AudioError> {
        let mut inner = Encoder::new(24_000, Channels::Mono, Application::Voip)
            .map_err(|_| AudioError::EncoderInit)?;
        inner
            .set_bitrate(Bitrate::Bits(32_000))
            .and_then(|_| inner.set_vbr(true))
            .and_then(|_| inner.set_vbr_constraint(true))
            .and_then(|_| inner.set_dtx(false))
            .and_then(|_| inner.set_inband_fec(false))
            .and_then(|_| inner.set_packet_loss_perc(0))
            .and_then(|_| inner.set_complexity(10))
            .map_err(|_| AudioError::EncoderInit)?;
        Ok(Self {
            inner,
            scratch: [0; DOWNLINK_ENCODE_BUFFER_BYTES],
            max_packet_bytes,
        })
    }

    pub fn encode(&mut self, frame: DownlinkPcmFrame) -> Result<OpusPacket, AudioError> {
        let encoded = self
            .inner
            .encode(frame.samples(), &mut self.scratch)
            .map_err(|_| AudioError::EncodeFailed)?;
        if encoded == 0 {
            return Err(AudioError::EmptyEncodedPacket);
        }
        if encoded > self.max_packet_bytes {
            return Err(AudioError::EncodedPacketTooLarge {
                actual: encoded,
                max: self.max_packet_bytes,
            });
        }
        Ok(OpusPacket(self.scratch[..encoded].to_vec()))
    }
}

const _: () = assert!(DOWNLINK_FRAME_SAMPLES == 1_440);
