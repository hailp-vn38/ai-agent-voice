use ring::rand::{SecureRandom, SystemRandom};

/// Low-order points on Curve25519 per RFC 7748.
pub(in crate::providers::llm::gateway::muse) const LOW_ORDER_POINTS: [[u8; 32]; 7] = [
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
