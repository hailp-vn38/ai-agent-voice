use ring::aead::{AES_256_GCM, Aad, LessSafeKey, Nonce, UnboundKey};
use ring::digest::{SHA256, digest};
use ring::hmac;
use ring::rand::{SecureRandom, SystemRandom};

const PROTOCOL_NAME: &[u8] = b"Noise_XX_25519_AESGCM_SHA256";
const MAX_NONCE: u64 = u64::MAX - 1;

/// Low-order points on Curve25519 per RFC 7748.
pub(super) const LOW_ORDER_POINTS: [[u8; 32]; 7] = [
    [0u8; 32],
    [
        1, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0,
        0, 0,
    ],
    [
        0xe0, 0xeb, 0x7a, 0x7c, 0x3b, 0x41, 0xb8, 0xae, 0x16, 0x56, 0xe3, 0xfa, 0xf1, 0x9f, 0xc4,
        0x6a, 0xda, 0x09, 0x8d, 0xeb, 0x9c, 0x32, 0xb1, 0xfd, 0x86, 0x62, 0x05, 0x16, 0x5f, 0x49,
        0xb8, 0x00,
    ],
    [
        0x5f, 0x9c, 0x95, 0xbc, 0xa3, 0x50, 0x8c, 0x24, 0xb1, 0xd0, 0xb1, 0x55, 0x9c, 0x83, 0xef,
        0x5b, 0x04, 0x44, 0x5c, 0xc4, 0x58, 0x1c, 0x8e, 0x86, 0xd8, 0x22, 0x4e, 0xdd, 0xd0, 0x9f,
        0x11, 0x57,
    ],
    [
        0xec, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff,
        0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff,
        0xff, 0x7f,
    ],
    [
        0xed, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff,
        0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff,
        0xff, 0x7f,
    ],
    [
        0xee, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff,
        0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff,
        0xff, 0x7f,
    ],
];

#[derive(Clone)]
pub(crate) struct X25519KeyPair {
    pub private_key: [u8; 32],
    pub public_key: [u8; 32],
}

impl X25519KeyPair {
    pub fn generate() -> Result<Self, String> {
        let rng = SystemRandom::new();
        let mut secret = [0u8; 32];
        rng.fill(&mut secret)
            .map_err(|_| "failed to generate X25519 random bytes".to_string())?;
        Self::from_secret(secret)
    }

    pub fn from_secret(mut secret: [u8; 32]) -> Result<Self, String> {
        secret[0] &= 248;
        secret[31] &= 127;
        secret[31] |= 64;
        let mut basepoint = [0u8; 32];
        basepoint[0] = 9;
        let public_key = x25519_raw(&secret, &basepoint);
        Ok(Self {
            private_key: secret,
            public_key,
        })
    }
}

pub(crate) fn x25519_dh(
    private_key: &[u8; 32],
    peer_public: &[u8; 32],
) -> Result<[u8; 32], String> {
    if LOW_ORDER_POINTS.iter().any(|p| p == peer_public) {
        return Err("Noise_XX: rejected low-order X25519 public key".to_string());
    }
    let shared = x25519_raw(private_key, peer_public);
    if shared.iter().all(|&b| b == 0) {
        return Err("Noise_XX: X25519 DH produced all-zero shared secret".to_string());
    }
    Ok(shared)
}

/// Montgomery ladder scalar multiplication on GF(2^255 - 19) per RFC 7748.
fn x25519_raw(scalar: &[u8; 32], u_bytes: &[u8; 32]) -> [u8; 32] {
    type Fe = [i64; 16];

    fn fe_from_bytes(b: &[u8; 32]) -> Fe {
        let mut out = [0i64; 16];
        for i in 0..16 {
            out[i] = (b[2 * i] as i64) | ((b[2 * i + 1] as i64) << 8);
        }
        out[15] &= 0x7fff;
        out
    }

    fn fe_carry(o: &mut Fe) {
        for i in 0..16 {
            let carry = o[i] >> 16;
            o[i] -= carry << 16;
            if i < 15 {
                o[i + 1] += carry;
            } else {
                o[0] += 38 * carry;
            }
        }
    }

    fn fe_add(a: &Fe, b: &Fe) -> Fe {
        let mut out = [0i64; 16];
        for i in 0..16 {
            out[i] = a[i] + b[i];
        }
        out
    }

    fn fe_sub(a: &Fe, b: &Fe) -> Fe {
        let mut out = [0i64; 16];
        for i in 0..16 {
            out[i] = a[i] - b[i];
        }
        out
    }

    fn fe_mul(a: &Fe, b: &Fe) -> Fe {
        let mut t = [0i64; 31];
        for i in 0..16 {
            for j in 0..16 {
                t[i + j] += a[i] * b[j];
            }
        }
        for i in 0..15 {
            t[i] += 38 * t[i + 16];
        }
        let mut out = [0i64; 16];
        out.copy_from_slice(&t[..16]);
        fe_carry(&mut out);
        fe_carry(&mut out);
        out
    }

    fn fe_sq(a: &Fe) -> Fe {
        fe_mul(a, a)
    }

    fn fe_inv(inp: &Fe) -> Fe {
        let mut c = *inp;
        for i in (0..=253).rev() {
            c = fe_sq(&c);
            if i != 2 && i != 4 {
                c = fe_mul(&c, inp);
            }
        }
        c
    }

    fn cswap(p: &mut Fe, q: &mut Fe, b: i64) {
        let mask = !(b - 1);
        for i in 0..16 {
            let t = mask & (p[i] ^ q[i]);
            p[i] ^= t;
            q[i] ^= t;
        }
    }

    fn fe_to_bytes(inp: &Fe) -> [u8; 32] {
        let mut t = *inp;
        fe_carry(&mut t);
        fe_carry(&mut t);
        fe_carry(&mut t);
        for _ in 0..2 {
            let mut m = [0i64; 16];
            m[0] = t[0] - 0xffed;
            for i in 1..15 {
                m[i] = t[i] - 0xffff - ((m[i - 1] >> 16) & 1);
                m[i - 1] &= 0xffff;
            }
            m[15] = t[15] - 0x7fff - ((m[14] >> 16) & 1);
            let b = (m[15] >> 16) & 1;
            m[14] &= 0xffff;
            cswap(&mut t, &mut m, 1 - b);
        }
        let mut out = [0u8; 32];
        for i in 0..16 {
            out[2 * i] = (t[i] & 0xff) as u8;
            out[2 * i + 1] = ((t[i] >> 8) & 0xff) as u8;
        }
        out
    }

    let mut k = *scalar;
    k[0] &= 248;
    k[31] &= 127;
    k[31] |= 64;

    let x_1 = fe_from_bytes(u_bytes);
    let mut x_2 = [0i64; 16];
    x_2[0] = 1;
    let mut z_2 = [0i64; 16];
    let mut x_3 = x_1;
    let mut z_3 = [0i64; 16];
    z_3[0] = 1;
    let mut a24 = [0i64; 16];
    a24[0] = 0xdb41;
    a24[1] = 1;

    let mut swap = 0i64;
    for pos in (0..=254).rev() {
        let b = ((k[pos / 8] >> (pos & 7)) & 1) as i64;
        swap ^= b;
        cswap(&mut x_2, &mut x_3, swap);
        cswap(&mut z_2, &mut z_3, swap);
        swap = b;

        let a = fe_add(&x_2, &z_2);
        let aa = fe_sq(&a);
        let b_fe = fe_sub(&x_2, &z_2);
        let bb = fe_sq(&b_fe);
        let e = fe_sub(&aa, &bb);
        let c = fe_add(&x_3, &z_3);
        let d = fe_sub(&x_3, &z_3);
        let da = fe_mul(&d, &a);
        let cb = fe_mul(&c, &b_fe);
        x_3 = fe_sq(&fe_add(&da, &cb));
        z_3 = fe_mul(&x_1, &fe_sq(&fe_sub(&da, &cb)));
        x_2 = fe_mul(&aa, &bb);
        z_2 = fe_mul(&e, &fe_add(&aa, &fe_mul(&a24, &e)));
    }
    cswap(&mut x_2, &mut x_3, swap);
    cswap(&mut z_2, &mut z_3, swap);

    let z_inv = fe_inv(&z_2);
    fe_to_bytes(&fe_mul(&x_2, &z_inv))
}

fn sha256(data: &[u8]) -> [u8; 32] {
    let d = digest(&SHA256, data);
    let mut out = [0u8; 32];
    out.copy_from_slice(d.as_ref());
    out
}

fn hmac_sha256(key: &[u8], data: &[u8]) -> [u8; 32] {
    let k = hmac::Key::new(hmac::HMAC_SHA256, key);
    let tag = hmac::sign(&k, data);
    let mut out = [0u8; 32];
    out.copy_from_slice(tag.as_ref());
    out
}

fn hkdf2(chaining_key: &[u8; 32], input_key_material: &[u8]) -> ([u8; 32], [u8; 32]) {
    let temp_key = hmac_sha256(chaining_key, input_key_material);
    let out1 = hmac_sha256(&temp_key, &[1u8]);
    let mut in2 = [0u8; 33];
    in2[..32].copy_from_slice(&out1);
    in2[32] = 2u8;
    let out2 = hmac_sha256(&temp_key, &in2);
    (out1, out2)
}

fn make_nonce(n: u64) -> [u8; 12] {
    let mut nonce = [0u8; 12];
    nonce[4..12].copy_from_slice(&n.to_be_bytes());
    nonce
}

pub(crate) struct CipherState {
    key: Option<LessSafeKey>,
    n: u64,
}

impl CipherState {
    pub fn new(key_bytes: Option<[u8; 32]>) -> Result<Self, String> {
        let key = match key_bytes {
            Some(kb) => {
                let unbound = UnboundKey::new(&AES_256_GCM, &kb)
                    .map_err(|_| "invalid AES-256-GCM key".to_string())?;
                Some(LessSafeKey::new(unbound))
            }
            None => None,
        };
        Ok(Self { key, n: 0 })
    }

    pub fn encrypt_with_ad(&mut self, ad: &[u8], plaintext: &[u8]) -> Result<Vec<u8>, String> {
        let Some(ref key) = self.key else {
            return Ok(plaintext.to_vec());
        };
        if self.n >= MAX_NONCE {
            return Err("Noise_XX: cipher nonce exhausted; reconnect required".to_string());
        }
        let nonce = Nonce::assume_unique_for_key(make_nonce(self.n));
        self.n += 1;
        let mut in_out = Vec::with_capacity(plaintext.len() + 16);
        in_out.extend_from_slice(plaintext);
        key.seal_in_place_append_tag(nonce, Aad::from(ad), &mut in_out)
            .map_err(|_| "Noise_XX: AES-GCM seal failed".to_string())?;
        Ok(in_out)
    }

    pub fn decrypt_with_ad(&mut self, ad: &[u8], ciphertext: &[u8]) -> Result<Vec<u8>, String> {
        let Some(ref key) = self.key else {
            return Ok(ciphertext.to_vec());
        };
        if ciphertext.len() < 16 {
            return Err("Noise_XX: ciphertext too short for GCM tag".to_string());
        }
        if self.n >= MAX_NONCE {
            return Err("Noise_XX: cipher nonce exhausted; reconnect required".to_string());
        }
        let nonce = Nonce::assume_unique_for_key(make_nonce(self.n));
        self.n += 1;
        let mut in_out = ciphertext.to_vec();
        let plaintext = key
            .open_in_place(nonce, Aad::from(ad), &mut in_out)
            .map_err(|_| "Noise_XX: AES-GCM authentication failed".to_string())?;
        Ok(plaintext.to_vec())
    }
}

struct SymmetricState {
    ck: [u8; 32],
    h: [u8; 32],
    cipher: CipherState,
}

impl SymmetricState {
    fn new(protocol_name: &[u8]) -> Result<Self, String> {
        let h = if protocol_name.len() <= 32 {
            let mut buf = [0u8; 32];
            buf[..protocol_name.len()].copy_from_slice(protocol_name);
            buf
        } else {
            sha256(protocol_name)
        };
        Ok(Self {
            ck: h,
            h,
            cipher: CipherState::new(None)?,
        })
    }

    fn mix_key(&mut self, input_key_material: &[u8]) -> Result<(), String> {
        let (ck, temp_k) = hkdf2(&self.ck, input_key_material);
        self.ck = ck;
        self.cipher = CipherState::new(Some(temp_k))?;
        Ok(())
    }

    fn mix_hash(&mut self, data: &[u8]) {
        let mut combined = Vec::with_capacity(32 + data.len());
        combined.extend_from_slice(&self.h);
        combined.extend_from_slice(data);
        self.h = sha256(&combined);
    }

    fn encrypt_and_hash(&mut self, plaintext: &[u8]) -> Result<Vec<u8>, String> {
        let ct = self.cipher.encrypt_with_ad(&self.h, plaintext)?;
        self.mix_hash(&ct);
        Ok(ct)
    }

    fn decrypt_and_hash(&mut self, ciphertext: &[u8]) -> Result<Vec<u8>, String> {
        let pt = self.cipher.decrypt_with_ad(&self.h, ciphertext)?;
        self.mix_hash(ciphertext);
        Ok(pt)
    }

    fn split(&self) -> Result<(CipherState, CipherState), String> {
        let (k1, k2) = hkdf2(&self.ck, &[]);
        Ok((CipherState::new(Some(k1))?, CipherState::new(Some(k2))?))
    }
}

/// 3-step `Noise_XX_25519_AESGCM_SHA256` handshake initiator.
pub(crate) struct NoiseXXInitiator {
    ss: SymmetricState,
    s: X25519KeyPair,
    e: Option<X25519KeyPair>,
    re: Option<[u8; 32]>,
    rs: Option<[u8; 32]>,
}

impl NoiseXXInitiator {
    pub fn new(static_keypair: Option<X25519KeyPair>) -> Result<Self, String> {
        let mut ss = SymmetricState::new(PROTOCOL_NAME)?;
        ss.mix_hash(&[]);
        let s = match static_keypair {
            Some(kp) => kp,
            None => X25519KeyPair::generate()?,
        };
        Ok(Self {
            ss,
            s,
            e: None,
            re: None,
            rs: None,
        })
    }

    /// Step 1 (`-> e`): Generates ephemeral key `e` and appends 34-byte protobuf client nonce.
    /// Total output is 66 bytes (`32B e.pub || 2B proto header [10, 32] || 32B nonce`).
    pub fn write_message_1(&mut self) -> Result<Vec<u8>, String> {
        let e = X25519KeyPair::generate()?;
        self.ss.mix_hash(&e.public_key);

        let rng = SystemRandom::new();
        let mut nonce = [0u8; 32];
        rng.fill(&mut nonce)
            .map_err(|_| "failed to generate Noise msg1 nonce".to_string())?;
        let mut payload = Vec::with_capacity(34);
        payload.extend_from_slice(&[10u8, 32u8]);
        payload.extend_from_slice(&nonce);

        let ct = self.ss.encrypt_and_hash(&payload)?;
        let mut out = Vec::with_capacity(32 + ct.len());
        out.extend_from_slice(&e.public_key);
        out.extend_from_slice(&ct);
        self.e = Some(e);
        Ok(out)
    }

    /// Step 2 (`<- e, ee, s, es`): Decrypts server ephemeral key `re`, static key `rs`, and payload.
    pub fn read_message_2(&mut self, msg: &[u8]) -> Result<Vec<u8>, String> {
        if msg.len() < 96 {
            return Err(format!(
                "Noise_XX msg2 too short: {} bytes (minimum 96)",
                msg.len()
            ));
        }
        let e = self.e.as_ref().ok_or_else(|| {
            "Noise_XX: write_message_1 must be called before read_message_2".to_string()
        })?;

        let mut re = [0u8; 32];
        re.copy_from_slice(&msg[..32]);
        self.ss.mix_hash(&re);

        let ee = x25519_dh(&e.private_key, &re)?;
        self.ss.mix_key(&ee)?;

        let rs_bytes = self.ss.decrypt_and_hash(&msg[32..80])?;
        if rs_bytes.len() != 32 {
            return Err("Noise_XX: decrypted server static key is not 32 bytes".to_string());
        }
        let mut rs = [0u8; 32];
        rs.copy_from_slice(&rs_bytes);

        let es = x25519_dh(&e.private_key, &rs)?;
        self.ss.mix_key(&es)?;

        let payload = self.ss.decrypt_and_hash(&msg[80..])?;
        self.re = Some(re);
        self.rs = Some(rs);
        Ok(payload)
    }

    /// Step 3 (`-> s, se`): Sends encrypted static key `s.pub` (48B) + empty payload tag (16B) = 64 bytes,
    /// then splits symmetric state into `(send_cipher, recv_cipher)`.
    pub fn write_message_3(&mut self) -> Result<(Vec<u8>, CipherState, CipherState), String> {
        let re = self.re.ok_or_else(|| {
            "Noise_XX: read_message_2 must be called before write_message_3".to_string()
        })?;

        let s_enc = self.ss.encrypt_and_hash(&self.s.public_key)?;
        let se = x25519_dh(&self.s.private_key, &re)?;
        self.ss.mix_key(&se)?;
        let ct = self.ss.encrypt_and_hash(&[])?;

        let mut msg3 = Vec::with_capacity(s_enc.len() + ct.len());
        msg3.extend_from_slice(&s_enc);
        msg3.extend_from_slice(&ct);

        let (send_cipher, recv_cipher) = self.ss.split()?;
        Ok((msg3, send_cipher, recv_cipher))
    }
}

#[cfg(test)]
pub(crate) struct NoiseXXResponder {
    ss: SymmetricState,
    s: X25519KeyPair,
    e: Option<X25519KeyPair>,
    re: Option<[u8; 32]>,
    rs: Option<[u8; 32]>,
}

#[cfg(test)]
impl NoiseXXResponder {
    pub fn new(static_keypair: Option<X25519KeyPair>) -> Result<Self, String> {
        let mut ss = SymmetricState::new(PROTOCOL_NAME)?;
        ss.mix_hash(&[]);
        let s = match static_keypair {
            Some(kp) => kp,
            None => X25519KeyPair::generate()?,
        };
        Ok(Self {
            ss,
            s,
            e: None,
            re: None,
            rs: None,
        })
    }

    pub fn read_message_1(&mut self, msg: &[u8]) -> Result<Vec<u8>, String> {
        if msg.len() < 32 {
            return Err("Noise_XX msg1 too short".to_string());
        }
        let mut re = [0u8; 32];
        re.copy_from_slice(&msg[..32]);
        self.ss.mix_hash(&re);
        let payload = self.ss.decrypt_and_hash(&msg[32..])?;
        self.re = Some(re);
        Ok(payload)
    }

    pub fn write_message_2(&mut self, payload: &[u8]) -> Result<Vec<u8>, String> {
        let re = self.re.ok_or_else(|| "missing re".to_string())?;
        let e = X25519KeyPair::generate()?;
        self.ss.mix_hash(&e.public_key);

        let ee = x25519_dh(&e.private_key, &re)?;
        self.ss.mix_key(&ee)?;

        let s_enc = self.ss.encrypt_and_hash(&self.s.public_key)?;
        let es = x25519_dh(&self.s.private_key, &re)?;
        self.ss.mix_key(&es)?;

        let payload_enc = self.ss.encrypt_and_hash(payload)?;
        let mut out = Vec::with_capacity(32 + s_enc.len() + payload_enc.len());
        out.extend_from_slice(&e.public_key);
        out.extend_from_slice(&s_enc);
        out.extend_from_slice(&payload_enc);
        self.e = Some(e);
        Ok(out)
    }

    pub fn read_message_3(
        &mut self,
        msg: &[u8],
    ) -> Result<(Vec<u8>, CipherState, CipherState), String> {
        if msg.len() < 64 {
            return Err("Noise_XX msg3 too short".to_string());
        }
        let e = self.e.as_ref().ok_or_else(|| "missing e".to_string())?;
        let rs_bytes = self.ss.decrypt_and_hash(&msg[..48])?;
        let mut rs = [0u8; 32];
        rs.copy_from_slice(&rs_bytes);
        let se = x25519_dh(&e.private_key, &rs)?;
        self.ss.mix_key(&se)?;
        let payload = self.ss.decrypt_and_hash(&msg[48..])?;
        self.rs = Some(rs);
        let (recv_cipher, send_cipher) = self.ss.split()?;
        Ok((payload, send_cipher, recv_cipher))
    }
}
