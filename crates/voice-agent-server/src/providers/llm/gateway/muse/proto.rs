use std::collections::HashMap;
use std::time::{Duration, Instant};

pub(crate) const OUTBOUND_CHUNK_SIZE: usize = 32_000;
pub(crate) const SERVICE_DAEMON: u64 = 0;
pub(crate) const SERVICE_SENTINEL: u64 = 1;

const MAX_PENDING_FRAMES: usize = 256;
const FRAME_TTL: Duration = Duration::from_secs(60);

pub(crate) fn encode_varint(mut value: u64) -> Vec<u8> {
    let mut out = Vec::with_capacity(10);
    while value > 0x7f {
        out.push(((value as u8) & 0x7f) | 0x80);
        value >>= 7;
    }
    out.push(value as u8);
    out
}

pub(crate) fn decode_varint(buf: &[u8], offset: usize) -> Result<(u64, usize), String> {
    let mut result = 0u64;
    let mut shift = 0u32;
    let mut pos = offset;
    while pos < buf.len() {
        let b = buf[pos];
        pos += 1;
        result |= ((b & 0x7f) as u64) << shift;
        if (b & 0x80) == 0 {
            return Ok((result, pos));
        }
        shift += 7;
        if shift > 63 {
            return Err("protobuf varint overflow".to_string());
        }
    }
    Err("unexpected EOF while decoding protobuf varint".to_string())
}

pub(crate) fn encode_varint_field(field_no: u32, value: u64) -> Vec<u8> {
    let mut out = encode_varint((field_no as u64) << 3);
    out.extend_from_slice(&encode_varint(value));
    out
}

pub(crate) fn encode_bytes_field(field_no: u32, data: &[u8]) -> Vec<u8> {
    let mut out = encode_varint(((field_no as u64) << 3) | 2);
    out.extend_from_slice(&encode_varint(data.len() as u64));
    out.extend_from_slice(data);
    out
}

pub(crate) fn encode_string_field(field_no: u32, text: &str) -> Vec<u8> {
    encode_bytes_field(field_no, text.as_bytes())
}

pub(crate) fn encode_header(key: &str, value: &str) -> Vec<u8> {
    let mut out = encode_string_field(1, key);
    out.extend_from_slice(&encode_string_field(2, value));
    out
}

pub(crate) fn encode_application_request(
    verb: &str,
    path: &str,
    headers: &[(&str, &str)],
    body: Option<&[u8]>,
    end_body: bool,
) -> Vec<u8> {
    let mut out = encode_string_field(1, verb);
    out.extend_from_slice(&encode_string_field(2, path));
    for (k, v) in headers {
        let hdr = encode_header(k, v);
        out.extend_from_slice(&encode_bytes_field(3, &hdr));
    }
    if let Some(b) = body
        && !b.is_empty()
    {
        out.extend_from_slice(&encode_bytes_field(4, b));
    }
    if end_body {
        out.extend_from_slice(&encode_varint_field(5, 1));
    }
    out
}

pub(crate) fn encode_service_frame(stream_id: u64, request_bytes: &[u8]) -> Vec<u8> {
    let mut out = encode_varint_field(1, stream_id);
    out.extend_from_slice(&encode_bytes_field(2, request_bytes));
    out
}

pub(crate) fn encode_service_request(service_type: u64, frame_bytes: &[u8]) -> Vec<u8> {
    let mut out = Vec::new();
    if service_type != 0 {
        out.extend_from_slice(&encode_varint_field(1, service_type));
    }
    out.extend_from_slice(&encode_bytes_field(2, frame_bytes));
    out
}

pub(crate) fn encode_noise_transport_frame(
    chunk_id: u64,
    chunk_index: u64,
    total_chunks: u64,
    payload: &[u8],
) -> Vec<u8> {
    let mut out = encode_varint_field(1, chunk_id);
    if chunk_index != 0 {
        out.extend_from_slice(&encode_varint_field(2, chunk_index));
    }
    out.extend_from_slice(&encode_varint_field(3, total_chunks));
    if !payload.is_empty() {
        out.extend_from_slice(&encode_bytes_field(4, payload));
    }
    out
}

/// Splits a `ServiceRequest` into one or more `NoiseTransportFrame`s capped at 32 KB each.
pub(crate) fn split_into_transport_frames(chunk_id: u64, service_req: &[u8]) -> Vec<Vec<u8>> {
    let total_chunks = service_req.len().max(1).div_ceil(OUTBOUND_CHUNK_SIZE) as u64;
    let mut frames = Vec::with_capacity(total_chunks as usize);
    for idx in 0..total_chunks {
        let start = (idx as usize) * OUTBOUND_CHUNK_SIZE;
        let end = (start + OUTBOUND_CHUNK_SIZE)
            .min(service_req.len())
            .max(start);
        let slice = if service_req.is_empty() {
            &[][..]
        } else {
            &service_req[start..end]
        };
        frames.push(encode_noise_transport_frame(
            chunk_id,
            idx,
            total_chunks,
            slice,
        ));
    }
    frames
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum ProtoValue {
    Varint(u64),
    Fixed64(u64),
    Bytes(Vec<u8>),
    Fixed32(u32),
}

#[derive(Debug, Default, Clone)]
pub(crate) struct ProtoFieldMap {
    fields: HashMap<u32, Vec<ProtoValue>>,
}

impl ProtoFieldMap {
    pub fn first_varint(&self, field_no: u32) -> Option<u64> {
        self.fields.get(&field_no)?.iter().find_map(|v| match v {
            ProtoValue::Varint(n) => Some(*n),
            _ => None,
        })
    }

    pub fn first_bytes(&self, field_no: u32) -> Option<&[u8]> {
        self.fields.get(&field_no)?.iter().find_map(|v| match v {
            ProtoValue::Bytes(b) => Some(b.as_slice()),
            _ => None,
        })
    }
}

pub(crate) fn decode_proto_fields(buf: &[u8]) -> Result<ProtoFieldMap, String> {
    let mut pos = 0usize;
    let mut map = ProtoFieldMap::default();
    while pos < buf.len() {
        let (tag, next_pos) = decode_varint(buf, pos)?;
        pos = next_pos;
        let field_no = (tag >> 3) as u32;
        let wire_type = (tag & 0x07) as u8;
        let value = match wire_type {
            0 => {
                let (v, np) = decode_varint(buf, pos)?;
                pos = np;
                ProtoValue::Varint(v)
            }
            1 => {
                if pos + 8 > buf.len() {
                    return Err("protobuf fixed64 truncated".to_string());
                }
                let mut arr = [0u8; 8];
                arr.copy_from_slice(&buf[pos..pos + 8]);
                pos += 8;
                ProtoValue::Fixed64(u64::from_le_bytes(arr))
            }
            2 => {
                let (len, np) = decode_varint(buf, pos)?;
                let len = len as usize;
                if np + len > buf.len() {
                    return Err("protobuf length-delimited field truncated".to_string());
                }
                let data = buf[np..np + len].to_vec();
                pos = np + len;
                ProtoValue::Bytes(data)
            }
            5 => {
                if pos + 4 > buf.len() {
                    return Err("protobuf fixed32 truncated".to_string());
                }
                let mut arr = [0u8; 4];
                arr.copy_from_slice(&buf[pos..pos + 4]);
                pos += 4;
                ProtoValue::Fixed32(u32::from_le_bytes(arr))
            }
            other => return Err(format!("unsupported protobuf wire type: {other}")),
        };
        map.fields.entry(field_no).or_default().push(value);
    }
    Ok(map)
}

struct PendingMultiChunk {
    total: usize,
    received: usize,
    parts: Vec<Option<Vec<u8>>>,
    created_at: Instant,
}

/// Inbound `NoiseTransportFrame` reassembler with capacity bound and 60s TTL.
#[derive(Default)]
pub(crate) struct NoiseFrameDecoder {
    pending: HashMap<u64, PendingMultiChunk>,
}

impl NoiseFrameDecoder {
    pub fn push(&mut self, raw: &[u8]) -> Result<Option<Vec<u8>>, String> {
        let fields = decode_proto_fields(raw)?;
        let chunk_id = fields.first_varint(1).unwrap_or(0);
        let chunk_index = fields.first_varint(2).unwrap_or(0) as usize;
        let total_chunks = fields.first_varint(3).unwrap_or(1) as usize;
        let payload = fields.first_bytes(4).unwrap_or(&[]);

        if total_chunks <= 1 {
            return Ok(Some(payload.to_vec()));
        }
        if chunk_index >= total_chunks || total_chunks > 4096 {
            return Err(format!(
                "invalid NoiseTransportFrame chunk header: index={chunk_index}, total={total_chunks}"
            ));
        }

        self.evict_stale();
        let entry = self
            .pending
            .entry(chunk_id)
            .or_insert_with(|| PendingMultiChunk {
                total: total_chunks,
                received: 0,
                parts: vec![None; total_chunks],
                created_at: Instant::now(),
            });

        if entry.total != total_chunks {
            let expected = entry.total;
            self.pending.remove(&chunk_id);
            return Err(format!(
                "inconsistent NoiseTransportFrame total_chunks for chunk_id={chunk_id}: expected {expected}, got {total_chunks}"
            ));
        }

        if let Some(slot @ None) = entry.parts.get_mut(chunk_index) {
            *slot = Some(payload.to_vec());
            entry.received += 1;
        }

        if entry.received < entry.total {
            return Ok(None);
        }

        let completed = self.pending.remove(&chunk_id).expect("entry exists");
        let total_len: usize = completed
            .parts
            .iter()
            .map(|p| p.as_ref().map_or(0, Vec::len))
            .sum();
        let mut merged = Vec::with_capacity(total_len);
        for part in completed.parts.into_iter().flatten() {
            merged.extend_from_slice(&part);
        }
        Ok(Some(merged))
    }

    fn evict_stale(&mut self) {
        let now = Instant::now();
        self.pending
            .retain(|_, v| now.duration_since(v.created_at) < FRAME_TTL);
        if self.pending.len() >= MAX_PENDING_FRAMES
            && let Some((&oldest_key, _)) = self.pending.iter().min_by_key(|(_, v)| v.created_at)
        {
            self.pending.remove(&oldest_key);
        }
    }
}

#[derive(Debug, Clone)]
pub(crate) struct DecodedApplicationResponse {
    pub status: u32,
    pub body: Vec<u8>,
    pub end_body: bool,
}

#[derive(Debug, Clone)]
pub(crate) struct DecodedBodyChunk {
    pub data: Vec<u8>,
    pub end_body: bool,
}

#[derive(Debug, Clone)]
pub(crate) struct DecodedServiceFrame {
    pub stream_id: u64,
    pub response: Option<DecodedApplicationResponse>,
    pub body_chunk: Option<DecodedBodyChunk>,
}

/// Decodes reassembled payload from `NoiseFrameDecoder` into `DecodedServiceFrame`.
pub(crate) fn decode_service_response_frame(
    assembled: &[u8],
) -> Result<Option<DecodedServiceFrame>, String> {
    let svc_resp = decode_proto_fields(assembled)?;
    let Some(frame_bytes) = svc_resp.first_bytes(1) else {
        return Ok(None);
    };
    let sf = decode_proto_fields(frame_bytes)?;
    let Some(stream_id) = sf.first_varint(1) else {
        return Ok(None);
    };

    let response = if let Some(resp_bytes) = sf.first_bytes(3) {
        let rf = decode_proto_fields(resp_bytes)?;
        Some(DecodedApplicationResponse {
            status: rf.first_varint(1).unwrap_or(200) as u32,
            body: rf.first_bytes(3).unwrap_or(&[]).to_vec(),
            end_body: rf.first_varint(4).unwrap_or(0) != 0,
        })
    } else {
        None
    };

    let body_chunk = if let Some(chunk_bytes) = sf.first_bytes(4) {
        let cf = decode_proto_fields(chunk_bytes)?;
        Some(DecodedBodyChunk {
            data: cf.first_bytes(1).unwrap_or(&[]).to_vec(),
            end_body: cf.first_varint(2).unwrap_or(0) != 0,
        })
    } else {
        None
    };

    Ok(Some(DecodedServiceFrame {
        stream_id,
        response,
        body_chunk,
    }))
}

#[cfg(test)]
#[derive(Debug, Clone)]
pub(crate) struct DecodedInboundRequest {
    pub service_type: u64,
    pub stream_id: u64,
    pub verb: String,
    pub path: String,
    pub body: Vec<u8>,
}

#[cfg(test)]
pub(crate) fn decode_service_request_frame(
    assembled: &[u8],
) -> Result<DecodedInboundRequest, String> {
    let svc_req = decode_proto_fields(assembled)?;
    let service_type = svc_req.first_varint(1).unwrap_or(0);
    let frame_bytes = svc_req
        .first_bytes(2)
        .ok_or_else(|| "missing ServiceFrame in ServiceRequest".to_string())?;
    let sf = decode_proto_fields(frame_bytes)?;
    let stream_id = sf
        .first_varint(1)
        .ok_or_else(|| "missing stream_id in ServiceFrame".to_string())?;
    let req_bytes = sf
        .first_bytes(2)
        .ok_or_else(|| "missing ApplicationRequest in ServiceFrame".to_string())?;
    let app_req = decode_proto_fields(req_bytes)?;
    let verb = String::from_utf8_lossy(app_req.first_bytes(1).unwrap_or(b"GET")).into_owned();
    let path = String::from_utf8_lossy(app_req.first_bytes(2).unwrap_or(b"/")).into_owned();
    let body = app_req.first_bytes(4).unwrap_or(&[]).to_vec();
    Ok(DecodedInboundRequest {
        service_type,
        stream_id,
        verb,
        path,
        body,
    })
}
