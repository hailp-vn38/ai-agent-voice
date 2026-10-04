//! Deterministic extraction for the pinned ContextBox tensor layout.
//! The pickle metadata is compared byte-for-byte, never interpreted or executed.
use super::ModelError;
use std::{
    collections::BTreeSet,
    io::{Cursor, Read},
};

const METADATA: &[u8] = include_bytes!("kokoro_tensor_v1.pkl");
const ROWS: u32 = 510;
const DIMENSION: u32 = 256;
const STORAGE_BYTES: usize = ROWS as usize * DIMENSION as usize * 4;
const MAX_ARCHIVE_BYTES: usize = 2 * 1024 * 1024;

pub(super) fn voicepack_v1(input: &[u8]) -> Result<Vec<u8>, ModelError> {
    let invalid = || {
        ModelError::UnsupportedTransform("kokoro_voicepack_v1: incompatible tensor archive".into())
    };
    if input.len() > MAX_ARCHIVE_BYTES {
        return Err(invalid());
    }
    let mut archive = zip::ZipArchive::new(Cursor::new(input)).map_err(|_| invalid())?;
    if archive.len() > 16 {
        return Err(invalid());
    }
    let mut names = BTreeSet::new();
    let mut metadata = None;
    let mut storage = None;
    let mut byteorder = None;
    let mut prefix = None;
    for index in 0..archive.len() {
        let mut entry = archive.by_index(index).map_err(|_| invalid())?;
        let name = entry.name().to_owned();
        if !names.insert(name.clone()) {
            return Err(invalid());
        }
        let (root, relative) = name.split_once('/').ok_or_else(invalid)?;
        if root.is_empty() || root == "." || root == ".." || root.contains('\\') {
            return Err(invalid());
        }
        if prefix.as_deref().is_some_and(|previous| previous != root) {
            return Err(invalid());
        }
        prefix = Some(root.to_owned());
        let (target, expected) = match relative {
            "data.pkl" => (&mut metadata, METADATA.len()),
            "byteorder" => (&mut byteorder, 6),
            "data/0" => (&mut storage, STORAGE_BYTES),
            ".format_version" | ".storage_alignment" | "version" | ".data/serialization_id" => {
                if entry.size() > 128 {
                    return Err(invalid());
                }
                continue;
            }
            _ => return Err(invalid()),
        };
        if entry.size() != expected as u64 {
            return Err(invalid());
        }
        let mut bytes = Vec::with_capacity(expected);
        (&mut entry)
            .take(expected as u64 + 1)
            .read_to_end(&mut bytes)?;
        if bytes.len() != expected {
            return Err(invalid());
        }
        *target = Some(bytes);
    }
    if metadata.as_deref() != Some(METADATA) || byteorder.as_deref() != Some(b"little") {
        return Err(invalid());
    }
    let storage = storage.ok_or_else(invalid)?;
    if storage
        .as_chunks::<4>()
        .0
        .iter()
        .any(|bytes| !f32::from_le_bytes(*bytes).is_finite())
    {
        return Err(invalid());
    }
    let mut output = b"KOVI_VOICEPACK_V1".to_vec();
    output.extend_from_slice(&ROWS.to_le_bytes());
    output.extend_from_slice(&1u32.to_le_bytes());
    output.extend_from_slice(&DIMENSION.to_le_bytes());
    output.extend_from_slice(&storage);
    Ok(output)
}
