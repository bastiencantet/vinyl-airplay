use anyhow::{Result, anyhow, ensure};
use chacha20poly1305::{
    ChaCha20Poly1305, KeyInit, Nonce,
    aead::{Aead, Payload},
};
use hkdf::Hkdf;
use num_bigint::BigUint;
use sha2::{Digest, Sha512};
use srp::{client::SrpClient, groups::G_3072};
use subtle::ConstantTimeEq;

pub fn derive(secret: &[u8], salt: &str, info: &str) -> [u8; 32] {
    let mut key = [0; 32];
    Hkdf::<Sha512>::new(Some(salt.as_bytes()), secret)
        .expand(info.as_bytes(), &mut key)
        .expect("valid HKDF output length");
    key
}

fn hash(parts: &[&[u8]]) -> Vec<u8> {
    let mut h = Sha512::new();
    for part in parts {
        h.update(part);
    }
    h.finalize().to_vec()
}

fn padded(number: &BigUint) -> Vec<u8> {
    let bytes = number.to_bytes_be();
    let mut result = vec![0; 384 - bytes.len()];
    result.extend_from_slice(&bytes);
    result
}

/// HAP SRP-6a, SHA-512, RFC5054 3072-bit group, with HAP's M1/M2 proofs.
/// The srp crate's high-level process_reply uses a DIFFERENT proof: do not use it.
pub struct HapSrp {
    pub public: Vec<u8>,
    pub proof: Vec<u8>,
    key: Vec<u8>,
    expected_server: Vec<u8>,
}

impl HapSrp {
    pub fn new(private: &[u8], salt: &[u8], server: &[u8], pin: &[u8]) -> Result<Self> {
        ensure!(
            server.len() <= 384 && !server.is_empty(),
            "invalid SRP public key length"
        );
        ensure!(
            !salt.is_empty() && salt.len() <= 64,
            "invalid SRP salt length"
        );
        let group = &*G_3072;
        let client = SrpClient::<Sha512>::new(group);
        let a = BigUint::from_bytes_be(private);
        let b = BigUint::from_bytes_be(server);
        ensure!(
            b != BigUint::default() && b < group.n,
            "invalid SRP server public value"
        );
        let a_pub = client.compute_a_pub(&a);
        let public = a_pub.to_bytes_be();
        let n = group.n.to_bytes_be();
        let g = group.g.to_bytes_be();
        let k = BigUint::from_bytes_be(&hash(&[&n, &padded(&group.g)]));
        let u = BigUint::from_bytes_be(&hash(&[&padded(&a_pub), &padded(&b)]));
        ensure!(u != BigUint::default(), "invalid SRP scrambling parameter");
        let identity = SrpClient::<Sha512>::compute_identity_hash(b"Pair-Setup", pin);
        let x = SrpClient::<Sha512>::compute_x(&identity, salt);
        let shared = client.compute_premaster_secret(&b, &k, &x, &a, &u);
        // HAP hashes the minimal big-endian S representation, as used by srptools.
        let key = hash(&[&shared.to_bytes_be()]);
        let hn = hash(&[&n]);
        let hg = hash(&[&g]);
        let xor: Vec<u8> = hn.iter().zip(hg).map(|(a, b)| a ^ b).collect();
        let proof = hash(&[
            &xor,
            &hash(&[b"Pair-Setup"]),
            salt,
            &public,
            &b.to_bytes_be(),
            &key,
        ]);
        let expected_server = hash(&[&public, &proof, &key]);
        Ok(Self {
            public,
            proof,
            key,
            expected_server,
        })
    }

    pub fn finish(self, server_proof: &[u8]) -> Result<Vec<u8>> {
        ensure!(
            self.expected_server.ct_eq(server_proof).unwrap_u8() == 1,
            "SRP server proof mismatch"
        );
        Ok(self.key)
    }
}

pub struct Cipher {
    cipher: ChaCha20Poly1305,
    counter: u64,
}

impl Cipher {
    pub fn new(key: &[u8; 32]) -> Self {
        Self {
            cipher: ChaCha20Poly1305::new(key.into()),
            counter: 0,
        }
    }

    fn nonce(&self) -> [u8; 12] {
        let mut nonce = [0; 12];
        nonce[4..].copy_from_slice(&self.counter.to_le_bytes());
        nonce
    }

    pub fn seal(&mut self, plain: &[u8], aad: &[u8]) -> Result<(Vec<u8>, [u8; 8])> {
        let nonce = self.nonce();
        let next = self
            .counter
            .checked_add(1)
            .ok_or_else(|| anyhow!("nonce exhausted"))?;
        let encrypted = self
            .cipher
            .encrypt(Nonce::from_slice(&nonce), Payload { msg: plain, aad })
            .map_err(|_| anyhow!("encryption failed"))?;
        self.counter = next;
        Ok((encrypted, nonce[4..].try_into()?))
    }

    pub fn open(&mut self, encrypted: &[u8], aad: &[u8]) -> Result<Vec<u8>> {
        let nonce = self.nonce();
        let next = self
            .counter
            .checked_add(1)
            .ok_or_else(|| anyhow!("nonce exhausted"))?;
        let plain = self
            .cipher
            .decrypt(
                Nonce::from_slice(&nonce),
                Payload {
                    msg: encrypted,
                    aad,
                },
            )
            .map_err(|_| anyhow!("authentication tag mismatch"))?;
        self.counter = next;
        Ok(plain)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn unhex(value: &str) -> Vec<u8> {
        (0..value.len())
            .step_by(2)
            .map(|i| u8::from_str_radix(&value[i..i + 2], 16).unwrap())
            .collect()
    }

    #[test]
    fn hap_srp_matches_independent_srptools_fixture() {
        let fixture: serde_json::Value =
            serde_json::from_str(include_str!("../tests/fixtures/hap_srp.json")).unwrap();
        let value = |key: &str| unhex(fixture[key].as_str().unwrap());
        let make =
            || HapSrp::new(&value("private"), &value("salt"), &value("server"), b"3939").unwrap();
        let srp = make();
        assert_eq!(srp.public, value("public"));
        assert_eq!(srp.proof, value("proof"));
        assert_eq!(srp.finish(&value("server_proof")).unwrap(), value("key"));
        assert!(make().finish(&[0; 64]).is_err());
    }
    #[test]
    fn authenticated_records_reject_tampering_and_replay() {
        let mut tx = Cipher::new(&[7; 32]);
        let mut rx = Cipher::new(&[7; 32]);
        let (sealed, nonce) = tx.seal(b"hello", &[5, 0]).unwrap();
        assert_eq!(nonce, [0; 8]);
        assert!(rx.open(&sealed, &[4, 0]).is_err());
        assert_eq!(rx.open(&sealed, &[5, 0]).unwrap(), b"hello");
        assert!(rx.open(&sealed, &[5, 0]).is_err());
        let (_, nonce) = tx.seal(b"world", &[5, 0]).unwrap();
        assert_eq!(nonce, 1u64.to_le_bytes());
    }
    #[test]
    fn rejects_invalid_srp_public_values() {
        for b in [vec![0], G_3072.n.to_bytes_be(), vec![1; 385]] {
            assert!(HapSrp::new(&[1; 32], &[2; 16], &b, b"3939").is_err());
        }
    }
}
