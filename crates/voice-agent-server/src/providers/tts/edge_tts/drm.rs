use std::sync::atomic::{AtomicI64, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

use chrono::DateTime;
use sha2::{Digest, Sha256};
use uuid::Uuid;

const WIN_EPOCH_SECONDS: i64 = 11_644_473_600;
const TICKS_PER_SECOND: i64 = 10_000_000;

static CLOCK_SKEW_SECONDS: AtomicI64 = AtomicI64::new(0);

const SHA256_INITIAL: [u32; 8] = [
    0x6A09_E667,
    0xBB67_AE85,
    0x3C6E_F372,
    0xA54F_F53A,
    0x510E_527F,
    0x9B05_688C,
    0x1F83_D9AB,
    0x5BE0_CD19,
];

const SHA256_STATE: [u32; 8] = [
    0xFD9F_BEA2,
    0x9124_3CE7,
    0xBE49_9094,
    0xCD2E_1E0A,
    0x6C00_6F49,
    0x04BD_35AB,
    0xFEB0_A7E4,
    0x03EF_FBD2,
];

const SHA256_SCHEDULE: [u32; 14] = [
    0xEB24_BF9E,
    0x1558_691F,
    0xA0C0_9EE2,
    0x0CFA_32BA,
    0xA622_EE8C,
    0xD1DD_9491,
    0xC0DD_25A0,
    0xF9E7_0AB6,
    0xF0C3_1573,
    0x7058_CE4E,
    0x7188_E606,
    0x00E5_AB33,
    0xD258_B060,
    0x7276_6564,
];

const SHA256_ROUND_CONSTANTS: [u32; 64] = [
    0x428A_2F98,
    0x7137_4491,
    0xB5C0_FBCF,
    0xE9B5_DBA5,
    0x3956_C25B,
    0x59F1_11F1,
    0x923F_82A4,
    0xAB1C_5ED5,
    0xD807_AA98,
    0x1283_5B01,
    0x2431_85BE,
    0x550C_7DC3,
    0x72BE_5D74,
    0x80DE_B1FE,
    0x9BDC_06A7,
    0xC19B_F174,
    0xE49B_69C1,
    0xEFBE_4786,
    0x0FC1_9DC6,
    0x240C_A1CC,
    0x2DE9_2C6F,
    0x4A74_84AA,
    0x5CB0_A9DC,
    0x76F9_88DA,
    0x983E_5152,
    0xA831_C66D,
    0xB003_27C8,
    0xBF59_7FC7,
    0xC6E0_0BF3,
    0xD5A7_9147,
    0x06CA_6351,
    0x1429_2967,
    0x27B7_0A85,
    0x2E1B_2138,
    0x4D2C_6DFC,
    0x5338_0D13,
    0x650A_7354,
    0x766A_0ABB,
    0x81C2_C92E,
    0x9272_2C85,
    0xA2BF_E8A1,
    0xA81A_664B,
    0xC24B_8B70,
    0xC76C_51A3,
    0xD192_E819,
    0xD699_0624,
    0xF40E_3585,
    0x106A_A070,
    0x19A4_C116,
    0x1E37_6C08,
    0x2748_774C,
    0x34B0_BCB5,
    0x391C_0CB3,
    0x4ED8_AA4A,
    0x5B9C_CA4F,
    0x682E_6FF3,
    0x748F_82EE,
    0x78A5_636F,
    0x84C8_7814,
    0x8CC7_0208,
    0x90BE_FFFA,
    0xA450_6CEB,
    0xBEF9_A3F7,
    0xC671_78F2,
];

const SHA256_ADJUSTMENT: u32 = 0xC9DC_06A7;
const SHA256_SIGMA0_ADJUSTMENT: u32 = 0x059C_0B80;
const SHA256_SIGMA1_ADJUSTMENT: u32 = 0x000B_92C0;

const CS64_COEFFICIENTS: [(u32, u32); 14] = [
    (0xE82E_91B1, 0xB327_00D0),
    (0x44DE_24C6, 0x88DF_F51D),
    (0x3BEB_65BC, 0x2951_4038),
    (0xC89C_5933, 0xB328_CBDF),
    (0x8D1E_C211, 0xD405_B4D5),
    (0x314E_F5D5, 0x26E5_5F3C),
    (0xE924_91B1, 0x6078_01C6),
    (0xF0A5_B9D5, 0x82B2_C1FB),
    (0x0606_4CF3, 0xFA51_A3DA),
    (0xDA79_FA60, 0x207C_C464),
    (0xBED2_0560, 0x3C3C_74F3),
    (0x89F9_C9F8, 0xD2D1_4B61),
    (0x35FA_38C6, 0xBB04_A2C1),
    (0xCB49_E3C6, 0xC562_6167),
];
const CS64_FACTOR: u32 = 0x8289_3933;
const CS64_ADJUSTMENT0: u32 = 0xF2F9_0B1A;
const CS64_ADJUSTMENT1: u32 = 0x0B1A_0000;

#[inline]
fn rol16(value: u32) -> u32 {
    value.rotate_left(16)
}

#[inline]
fn sigma0(value: u32) -> u32 {
    value.rotate_right(7) ^ value.rotate_right(18) ^ (value >> 3)
}

#[inline]
fn sigma1(value: u32) -> u32 {
    value.rotate_right(17) ^ value.rotate_right(19) ^ (value >> 10)
}

fn words_le4_even(data: &[u8]) -> Vec<u32> {
    if data.is_empty() {
        return Vec::new();
    }
    let mut padded = data.to_vec();
    let rem = padded.len() % 4;
    if rem != 0 {
        padded.resize(padded.len() + (4 - rem), 0);
    }
    let mut words: Vec<u32> = padded
        .chunks_exact(4)
        .map(|chunk| u32::from_le_bytes([chunk[0], chunk[1], chunk[2], chunk[3]]))
        .collect();
    if words.len() % 2 != 0 {
        words.push(0);
    }
    words
}

fn expand_sha256_schedule(words: &mut [u32; 64], first_word: usize) {
    for index in first_word..64 {
        let s1 = sigma1(words[index - 2]);
        let s0 = sigma0(words[index - 15]);
        words[index] = s1
            .wrapping_add(words[index - 7])
            .wrapping_add(s0)
            .wrapping_add(words[index - 16]);
    }
}

fn sha256_rounds(state: [u32; 8], words: &[u32; 64], first_round: usize) -> [u32; 8] {
    let mut a = state[0];
    let mut b = state[1];
    let mut c = state[2];
    let mut d = state[3];
    let mut e = state[4];
    let mut f = state[5];
    let mut g = state[6];
    let mut h = state[7];

    for index in first_round..64 {
        let k = if first_round == 14 && index == 14 {
            SHA256_ADJUSTMENT
        } else {
            SHA256_ROUND_CONSTANTS[index]
        };
        let big_sigma1 = e.rotate_right(6) ^ e.rotate_right(11) ^ e.rotate_right(25);
        let ch = g ^ (e & (f ^ g));
        let t1 = h
            .wrapping_add(big_sigma1)
            .wrapping_add(ch)
            .wrapping_add(k)
            .wrapping_add(words[index]);
        let big_sigma0 = a.rotate_right(2) ^ a.rotate_right(13) ^ a.rotate_right(22);
        let maj = (a & b) | (c & (a | b));
        let t2 = big_sigma0.wrapping_add(maj);

        h = g;
        g = f;
        f = e;
        e = d.wrapping_add(t1);
        d = c;
        c = b;
        b = a;
        a = t1.wrapping_add(t2);
    }

    [a, b, c, d, e, f, g, h]
}

fn sha256_init(salt: &[u8]) -> [u32; 8] {
    let mut words = [0u32; 64];
    words[14] = u32::from_be_bytes([0, salt[0], salt[1], salt[2]]);
    words[15] = u32::from_be_bytes([salt[3], salt[4], salt[5], salt[6]]);

    for index in 16..30 {
        let mut value = SHA256_SCHEDULE[index - 16];
        if index - 2 >= 14 {
            let s1 = if index == 16 {
                sigma1(words[index - 2]) ^ SHA256_SIGMA1_ADJUSTMENT
            } else {
                sigma1(words[index - 2])
            };
            value = value.wrapping_add(s1);
        }
        if index - 7 >= 14 {
            value = value.wrapping_add(words[index - 7]);
        }
        if index - 15 >= 14 {
            let s0 = if index == 29 {
                sigma0(words[index - 15]) ^ SHA256_SIGMA0_ADJUSTMENT
            } else {
                sigma0(words[index - 15])
            };
            value = value.wrapping_add(s0);
        }
        words[index] = value;
    }

    words[30] = sigma1(words[28])
        .wrapping_add(words[23])
        .wrapping_add(sigma0(words[15]))
        .wrapping_add(words[14])
        .wrapping_add(SHA256_ADJUSTMENT)
        .wrapping_sub(SHA256_ROUND_CONSTANTS[14]);

    expand_sha256_schedule(&mut words, 31);
    let working = sha256_rounds(SHA256_STATE, &words, 14);
    let mut out = [0u32; 8];
    for i in 0..8 {
        out[i] = SHA256_INITIAL[i].wrapping_add(working[i]);
    }
    out
}

fn sha256_digest(data: &[u8]) -> [u8; 32] {
    assert!(
        data.len() >= 7,
        "Token salt must contain at least seven bytes"
    );
    let length = 57 + data.len();
    let pad_zeros = (55usize.wrapping_sub(length)) % 64;
    let mut padded = Vec::with_capacity(data.len() - 7 + 1 + pad_zeros + 8);
    padded.extend_from_slice(&data[7..]);
    padded.push(0x80);
    padded.resize(padded.len() + pad_zeros, 0);
    padded.extend_from_slice(&((length as u64) * 8).to_be_bytes());

    let mut state = sha256_init(&data[..7]);
    for block in padded.chunks_exact(64) {
        let mut words = [0u32; 64];
        for (i, chunk) in block.chunks_exact(4).enumerate() {
            words[i] = u32::from_be_bytes([chunk[0], chunk[1], chunk[2], chunk[3]]);
        }
        expand_sha256_schedule(&mut words, 16);
        let working = sha256_rounds(state, &words, 0);
        for i in 0..8 {
            state[i] = state[i].wrapping_add(working[i]);
        }
    }

    let mut digest = [0u8; 32];
    for (i, word) in state.iter().enumerate() {
        digest[i * 4..(i + 1) * 4].copy_from_slice(&word.to_be_bytes());
    }
    digest
}

fn cs64_digest(data: &[u8], digest: &[u8; 32]) -> (u32, u32) {
    let key_a = u32::from_le_bytes([digest[0], digest[1], digest[2], digest[3]]) | 1;
    let key_b = u32::from_le_bytes([digest[4], digest[5], digest[6], digest[7]]) | 1;
    let factors = [
        key_a.wrapping_mul(CS64_FACTOR),
        key_b.wrapping_mul(CS64_FACTOR),
    ];

    let mut terms: Vec<u32> = CS64_COEFFICIENTS
        .iter()
        .enumerate()
        .map(|(index, (coefficient, _))| coefficient.wrapping_mul(factors[index % 2]))
        .collect();
    let mut offsets: Vec<u32> = CS64_COEFFICIENTS
        .iter()
        .map(|(_, offset)| *offset)
        .collect();

    let mut prefixed = Vec::with_capacity(1 + data.len());
    prefixed.push(0);
    prefixed.extend_from_slice(data);

    for (index, word) in words_le4_even(&prefixed).into_iter().enumerate() {
        let mut term = word.wrapping_mul(if index % 2 == 0 { key_a } else { key_b });
        let mut offset = rol16(word).wrapping_mul(if index % 2 == 0 {
            0xEF05_69FB
        } else {
            0xC317_13DB
        });
        if index == 0 {
            term = term.wrapping_add(CS64_ADJUSTMENT0.wrapping_mul(factors[0]));
            offset = offset.wrapping_add(CS64_ADJUSTMENT1);
        }
        terms.push(term);
        offsets.push(offset);
    }

    let mut swap_acc = 0u32;
    let mut swap_last = 0u32;
    let mut state = 0u32;
    let mut acc = 0u32;

    for index in (0..terms.len()).step_by(2) {
        let x = terms[index];
        let y = terms[index + 1];

        let t = rol16(offsets[index].wrapping_add(x));
        let q = t.wrapping_mul(0x76F5_0F34);
        swap_acc = swap_acc.wrapping_add(rol16(q).wrapping_mul(0xC3EF_EA97).wrapping_add(q));

        let t = rol16(offsets[index + 1].wrapping_add(y));
        let q = t.wrapping_mul(0x1881_CE3C);
        swap_last = rol16(q).wrapping_mul(0x35BD_1EC9).wrapping_add(q);
        swap_acc = swap_acc.wrapping_add(swap_last);

        state = state.wrapping_mul(key_a).wrapping_add(x);
        state = rol16(state).wrapping_mul(0xCF98_B111);
        state = rol16(state).wrapping_mul(0x8708_5B9F);
        state = rol16(state).wrapping_mul(0x12CE_B96D);
        state = rol16(state).wrapping_mul(0x257E_1D83);
        acc = acc.wrapping_add(state);

        let mut t = state.wrapping_mul(key_b).wrapping_add(y);
        t = rol16(t).wrapping_mul(0xA274_16F5);
        t = rol16(t).wrapping_mul(0xD383_96FF);
        state = rol16(rol16(t).wrapping_mul(0x7C93_2B89)).wrapping_mul(0xBFA4_9F69);
        acc = acc.wrapping_add(state);
    }

    (swap_last ^ state, swap_acc ^ acc)
}

fn encode_upper_hex(bytes: &[u8]) -> String {
    let mut out = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        use std::fmt::Write as _;
        let _ = write!(&mut out, "{byte:02X}");
    }
    out
}

pub fn generate_sec_ms_gec_at(trusted_client_token: &str, version: &str, unix_ts: f64) -> String {
    let mut seconds = unix_ts.floor() as i64 + WIN_EPOCH_SECONDS;
    seconds -= seconds.rem_euclid(300);
    let ticks = seconds * TICKS_PER_SECOND;
    let payload = format!("{ticks}{trusted_client_token}");
    let data = payload.as_bytes();

    if version.starts_with("1-") {
        let digest = Sha256::digest(data);
        return encode_upper_hex(&digest);
    }

    let digest = sha256_digest(data);
    let (cs0, cs1) = cs64_digest(data, &digest);
    let mut cs_bytes = [0u8; 8];
    cs_bytes[..4].copy_from_slice(&cs0.to_le_bytes());
    cs_bytes[4..].copy_from_slice(&cs1.to_le_bytes());
    format!(
        "{}-{}",
        encode_upper_hex(&cs_bytes),
        encode_upper_hex(&digest)
    )
}

pub fn current_unix_timestamp_with_skew() -> f64 {
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs_f64();
    now + (CLOCK_SKEW_SECONDS.load(Ordering::Relaxed) as f64)
}

pub fn generate_sec_ms_gec(trusted_client_token: &str, version: &str) -> String {
    generate_sec_ms_gec_at(
        trusted_client_token,
        version,
        current_unix_timestamp_with_skew(),
    )
}

pub fn adjust_clock_skew_from_rfc2822_date(server_date: &str) -> bool {
    let Ok(parsed) = DateTime::parse_from_rfc2822(server_date.trim()) else {
        return false;
    };
    let server_ts = parsed.timestamp();
    let client_now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs() as i64;
    CLOCK_SKEW_SECONDS.store(server_ts - client_now, Ordering::Relaxed);
    true
}

pub fn generate_muid() -> String {
    Uuid::new_v4().simple().to_string().to_uppercase()
}

#[cfg(test)]
mod tests {
    use super::*;

    const TOKEN: &str = "6A5AA1D4EAFF4E9FB37E23D68491D6F4";

    #[test]
    fn sec_ms_gec_v2_matches_reference_vectors() {
        assert_eq!(
            generate_sec_ms_gec_at(TOKEN, "2-143.0.3650.75", 1_700_000_000.0),
            "2CD23A2CA0B2595A-F3C846C370E05F9CC83ACC5F995C5F5D87A4D0F14CE8936A0F902E64099E38A2"
        );
        assert_eq!(
            generate_sec_ms_gec_at(TOKEN, "2-143.0.3650.75", 1_700_000_299.0),
            "0919602B5750900A-EC9DD394F23CB5635CF62A0E8E24136DCD32B26D10C543B10082E482D7516397"
        );
        assert_eq!(
            generate_sec_ms_gec_at(TOKEN, "2-143.0.3650.75", 1_761_400_000.0),
            "D96E7FBD4DD2970E-90C3494324F333E9CAD6C2FBF64C010C4986898CB1299C3A2724A9E25C61EF8E"
        );
    }

    #[test]
    fn sec_ms_gec_v1_uses_plain_sha256_fallback() {
        let v1 = generate_sec_ms_gec_at(TOKEN, "1-130.0.2849.68", 1_700_000_000.0);
        assert_eq!(v1.len(), 64);
        assert!(v1.chars().all(|c| c.is_ascii_hexdigit()));
    }

    #[test]
    fn rfc2822_date_adjusts_clock_skew() {
        assert!(adjust_clock_skew_from_rfc2822_date(
            "Sun, 25 Oct 2026 14:00:00 GMT"
        ));
        assert!(!adjust_clock_skew_from_rfc2822_date("not-a-valid-date"));
        CLOCK_SKEW_SECONDS.store(0, Ordering::Relaxed);
    }
}
