use serde::Serialize;

#[derive(Debug, Serialize)]
pub struct ServerHello<'a> {
    #[serde(rename = "type")]
    pub message_type: &'static str,
    pub transport: &'static str,
    pub session_id: &'a str,
    pub audio_params: ServerAudioParams,
}

#[derive(Debug, Serialize)]
pub struct ServerAudioParams {
    pub format: &'static str,
    pub sample_rate: u32,
    pub channels: u8,
    pub frame_duration: u16,
}

impl<'a> ServerHello<'a> {
    pub fn v1(session_id: &'a str) -> Self {
        Self {
            message_type: "hello",
            transport: "websocket",
            session_id,
            audio_params: ServerAudioParams {
                format: "opus",
                sample_rate: 24_000,
                channels: 1,
                frame_duration: 60,
            },
        }
    }
}

#[derive(Debug, Serialize)]
pub struct OtaResponse {
    pub server_time: ServerTime,
    pub firmware: Firmware,
    pub websocket: OtaWebsocket,
}

#[derive(Debug, Serialize)]
pub struct ServerTime {
    pub timestamp: i64,
    pub timezone_offset: i16,
}
#[derive(Debug, Serialize)]
pub struct Firmware {
    pub version: &'static str,
    pub url: &'static str,
}
#[derive(Debug, Serialize)]
pub struct OtaWebsocket {
    pub url: String,
    pub token: String,
}
