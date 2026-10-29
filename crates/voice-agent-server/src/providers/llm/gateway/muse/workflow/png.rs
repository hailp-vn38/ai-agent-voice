use std::collections::VecDeque;

use super::super::pool::UpstreamFailureKind;
use super::WorkflowFailure;

pub(crate) fn remove_background_to_rgba_png(bytes: &[u8]) -> Result<Vec<u8>, WorkflowFailure> {
    const PNG_SIG: &[u8; 8] = b"\x89PNG\r\n\x1a\n";
    let Some((w, h, mut rgba)) = decode_uncompressed_or_simple_png(bytes) else {
        if bytes.starts_with(PNG_SIG) {
            return Ok(bytes.to_vec());
        }
        return Err(WorkflowFailure::Upstream(
            UpstreamFailureKind::Transient,
            "failed to decode image for background removal".to_string(),
        ));
    };
    if w == 0 || h == 0 {
        return Ok(bytes.to_vec());
    }

    let pixel_at = |buf: &[u8], x: u32, y: u32| -> [u8; 4] {
        let i = ((y * w + x) * 4) as usize;
        [buf[i], buf[i + 1], buf[i + 2], buf[i + 3]]
    };

    let corners = [
        pixel_at(&rgba, 0, 0),
        pixel_at(&rgba, w - 1, 0),
        pixel_at(&rgba, 0, h - 1),
        pixel_at(&rgba, w - 1, h - 1),
    ];
    let bg = [
        ((corners[0][0] as u32
            + corners[1][0] as u32
            + corners[2][0] as u32
            + corners[3][0] as u32)
            / 4) as u8,
        ((corners[0][1] as u32
            + corners[1][1] as u32
            + corners[2][1] as u32
            + corners[3][1] as u32)
            / 4) as u8,
        ((corners[0][2] as u32
            + corners[1][2] as u32
            + corners[2][2] as u32
            + corners[3][2] as u32)
            / 4) as u8,
    ];

    let color_dist_sq = |p: [u8; 4]| -> u32 {
        let dr = p[0] as i32 - bg[0] as i32;
        let dg = p[1] as i32 - bg[1] as i32;
        let db = p[2] as i32 - bg[2] as i32;
        (dr * dr + dg * dg + db * db) as u32
    };

    let hard_thresh_sq = 32u32 * 32u32;
    let soft_thresh_sq = 64u32 * 64u32;
    let mut visited = vec![false; (w * h) as usize];
    let mut queue = VecDeque::new();

    for x in 0..w {
        queue.push_back((x, 0));
        queue.push_back((x, h - 1));
    }
    for y in 0..h {
        queue.push_back((0, y));
        queue.push_back((w - 1, y));
    }

    while let Some((x, y)) = queue.pop_front() {
        let idx = (y * w + x) as usize;
        if visited[idx] {
            continue;
        }
        let px = pixel_at(&rgba, x, y);
        let d = color_dist_sq(px);
        if d > soft_thresh_sq {
            continue;
        }
        visited[idx] = true;
        let alpha = if d <= hard_thresh_sq {
            0u8
        } else {
            let ratio = ((d - hard_thresh_sq) as f32) / ((soft_thresh_sq - hard_thresh_sq) as f32);
            (ratio.clamp(0.0, 1.0) * 255.0) as u8
        };
        let off = idx * 4;
        rgba[off + 3] = alpha;

        if x > 0 {
            queue.push_back((x - 1, y));
        }
        if x + 1 < w {
            queue.push_back((x + 1, y));
        }
        if y > 0 {
            queue.push_back((x, y - 1));
        }
        if y + 1 < h {
            queue.push_back((x, y + 1));
        }
    }

    Ok(encode_rgba_png(w, h, &rgba))
}

fn decode_uncompressed_or_simple_png(bytes: &[u8]) -> Option<(u32, u32, Vec<u8>)> {
    if !bytes.starts_with(b"\x89PNG\r\n\x1a\n") || bytes.len() < 33 {
        return None;
    }
    let mut pos = 8usize;
    let mut width = 0u32;
    let mut height = 0u32;
    let mut color_type = 0u8;
    let mut idat = Vec::new();

    while pos + 12 <= bytes.len() {
        let len = u32::from_be_bytes(bytes[pos..pos + 4].try_into().ok()?) as usize;
        let chunk_type = &bytes[pos + 4..pos + 8];
        if pos + 12 + len > bytes.len() {
            break;
        }
        let data = &bytes[pos + 8..pos + 8 + len];
        match chunk_type {
            b"IHDR" if data.len() >= 13 => {
                width = u32::from_be_bytes(data[0..4].try_into().ok()?);
                height = u32::from_be_bytes(data[4..8].try_into().ok()?);
                let bit_depth = data[8];
                color_type = data[9];
                if bit_depth != 8 || !matches!(color_type, 2 | 6) {
                    return None;
                }
            }
            b"IDAT" => idat.extend_from_slice(data),
            b"IEND" => break,
            _ => {}
        }
        pos += 12 + len;
    }

    if width == 0 || height == 0 || idat.len() < 7 {
        return None;
    }
    // Stored zlib block decoding (BTYPE=00)
    let mut raw = Vec::new();
    let mut zpos = 2usize;
    while zpos + 5 <= idat.len().saturating_sub(4) {
        let header = idat[zpos];
        let btype = (header >> 1) & 0x03;
        if btype != 0 {
            return None;
        }
        let blen = u16::from_le_bytes([idat[zpos + 1], idat[zpos + 2]]) as usize;
        zpos += 5;
        if zpos + blen > idat.len().saturating_sub(4) {
            return None;
        }
        raw.extend_from_slice(&idat[zpos..zpos + blen]);
        zpos += blen;
        if (header & 1) != 0 {
            break;
        }
    }

    let bpp = if color_type == 6 { 4usize } else { 3usize };
    let stride = 1 + (width as usize) * bpp;
    if raw.len() < stride * (height as usize) {
        return None;
    }
    let mut rgba = Vec::with_capacity((width * height * 4) as usize);
    for y in 0..(height as usize) {
        let row = &raw[y * stride..y * stride + stride];
        if row[0] != 0 {
            return None;
        }
        for x in 0..(width as usize) {
            let p = &row[1 + x * bpp..1 + (x + 1) * bpp];
            rgba.push(p[0]);
            rgba.push(p[1]);
            rgba.push(p[2]);
            rgba.push(if bpp == 4 { p[3] } else { 255 });
        }
    }
    Some((width, height, rgba))
}

pub(crate) fn encode_rgba_png(width: u32, height: u32, rgba: &[u8]) -> Vec<u8> {
    let mut raw = Vec::with_capacity(((width * 4 + 1) * height) as usize);
    for y in 0..height {
        raw.push(0u8); // filter type 0 (None)
        let start = (y * width * 4) as usize;
        let end = start + (width * 4) as usize;
        raw.extend_from_slice(&rgba[start..end]);
    }

    // Stored zlib stream
    let mut zlib = Vec::with_capacity(raw.len() + 32);
    zlib.push(0x78);
    zlib.push(0x01);
    let chunks: Vec<&[u8]> = if raw.is_empty() {
        vec![&[]]
    } else {
        raw.chunks(65_535).collect()
    };
    for (i, chunk) in chunks.iter().enumerate() {
        let is_last = i + 1 == chunks.len();
        zlib.push(if is_last { 0x01 } else { 0x00 });
        let len = chunk.len() as u16;
        let nlen = !len;
        zlib.extend_from_slice(&len.to_le_bytes());
        zlib.extend_from_slice(&nlen.to_le_bytes());
        zlib.extend_from_slice(chunk);
    }
    zlib.extend_from_slice(&adler32(&raw).to_be_bytes());

    let mut out = Vec::with_capacity(zlib.len() + 64);
    out.extend_from_slice(b"\x89PNG\r\n\x1a\n");

    let mut ihdr = Vec::with_capacity(13);
    ihdr.extend_from_slice(&width.to_be_bytes());
    ihdr.extend_from_slice(&height.to_be_bytes());
    ihdr.extend_from_slice(&[8, 6, 0, 0, 0]);
    write_png_chunk(&mut out, b"IHDR", &ihdr);
    write_png_chunk(&mut out, b"IDAT", &zlib);
    write_png_chunk(&mut out, b"IEND", &[]);
    out
}

fn write_png_chunk(out: &mut Vec<u8>, chunk_type: &[u8; 4], data: &[u8]) {
    out.extend_from_slice(&(data.len() as u32).to_be_bytes());
    out.extend_from_slice(chunk_type);
    out.extend_from_slice(data);
    let mut crc = 0xFFFF_FFFFu32;
    for &b in chunk_type.iter().chain(data.iter()) {
        crc ^= b as u32;
        for _ in 0..8 {
            crc = if (crc & 1) != 0 {
                (crc >> 1) ^ 0xEDB8_8320
            } else {
                crc >> 1
            };
        }
    }
    out.extend_from_slice(&(!crc).to_be_bytes());
}

fn adler32(data: &[u8]) -> u32 {
    let mut a = 1u32;
    let mut b = 0u32;
    for &byte in data {
        a = (a + byte as u32) % 65_521;
        b = (b + a) % 65_521;
    }
    (b << 16) | a
}
