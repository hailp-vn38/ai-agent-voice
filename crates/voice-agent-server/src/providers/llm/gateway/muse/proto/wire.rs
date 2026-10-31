use std::collections::HashMap;

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
