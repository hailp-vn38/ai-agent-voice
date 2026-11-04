use ring::aead::{AES_256_GCM, Aad, LessSafeKey, Nonce, UnboundKey};
use ring::digest::{SHA256, digest};
use ring::hmac;

const MAX_NONCE: u64 = u64::MAX - 1;

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

pub(super) struct SymmetricState {
    ck: [u8; 32],
    h: [u8; 32],
    cipher: CipherState,
}

impl SymmetricState {
    pub fn new(protocol_name: &[u8]) -> Result<Self, String> {
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

    pub fn mix_key(&mut self, input_key_material: &[u8]) -> Result<(), String> {
        let (ck, temp_k) = hkdf2(&self.ck, input_key_material);
        self.ck = ck;
        self.cipher = CipherState::new(Some(temp_k))?;
        Ok(())
    }

    pub fn mix_hash(&mut self, data: &[u8]) {
        let mut combined = Vec::with_capacity(32 + data.len());
        combined.extend_from_slice(&self.h);
        combined.extend_from_slice(data);
        self.h = sha256(&combined);
    }

    pub fn encrypt_and_hash(&mut self, plaintext: &[u8]) -> Result<Vec<u8>, String> {
        let ct = self.cipher.encrypt_with_ad(&self.h, plaintext)?;
        self.mix_hash(&ct);
        Ok(ct)
    }

    pub fn decrypt_and_hash(&mut self, ciphertext: &[u8]) -> Result<Vec<u8>, String> {
        let pt = self.cipher.decrypt_with_ad(&self.h, ciphertext)?;
        self.mix_hash(ciphertext);
        Ok(pt)
    }

    pub fn split(&self) -> Result<(CipherState, CipherState), String> {
        let (k1, k2) = hkdf2(&self.ck, &[]);
        Ok((CipherState::new(Some(k1))?, CipherState::new(Some(k2))?))
    }
}
