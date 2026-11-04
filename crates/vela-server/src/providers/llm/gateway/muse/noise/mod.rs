mod cipher;
mod x25519;

use ring::rand::{SecureRandom, SystemRandom};

pub(crate) use self::cipher::CipherState;
use self::cipher::SymmetricState;
#[cfg(test)]
pub(in crate::providers::llm::gateway::muse) use self::x25519::LOW_ORDER_POINTS;
pub(crate) use self::x25519::{X25519KeyPair, x25519_dh};

const PROTOCOL_NAME: &[u8] = b"Noise_XX_25519_AESGCM_SHA256";

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
