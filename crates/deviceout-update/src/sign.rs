use ed25519_dalek::{Signature, Signer, SigningKey, Verifier, VerifyingKey};

pub const PUBLIC_KEYS: [[u8; 32]; 2] = [
    [
        0x41, 0x76, 0x6e, 0x30, 0xf6, 0xf2, 0x74, 0xd2, 0x49, 0x3c, 0x7c, 0x6e, 0xda, 0xbc, 0xd0,
        0x19, 0xae, 0xea, 0xfd, 0xfd, 0x7f, 0x7d, 0x93, 0x0a, 0x1a, 0xd8, 0x9c, 0x0c, 0xe3, 0xd1,
        0x80, 0x90,
    ],
    [
        0x97, 0x19, 0x6a, 0xcd, 0x2d, 0x5f, 0x7c, 0xde, 0x02, 0x04, 0xae, 0xe7, 0x37, 0x38, 0x5e,
        0xd5, 0x37, 0xc7, 0x62, 0x2c, 0x8b, 0x84, 0xba, 0x78, 0x22, 0xcf, 0xb5, 0x3e, 0x3f, 0x63,
        0xf0, 0x28,
    ],
];

pub fn verify(message: &[u8], sig_bytes: &[u8]) -> bool {
    let sig_arr: [u8; 64] = match decode_sig(sig_bytes) {
        Some(s) => s,
        None => return false,
    };
    let sig = Signature::from_bytes(&sig_arr);
    for raw in PUBLIC_KEYS {
        let Ok(key) = VerifyingKey::from_bytes(&raw) else {
            continue;
        };
        if key.verify(message, &sig).is_ok() {
            return true;
        }
    }
    false
}

pub fn sign(secret32: &[u8; 32], message: &[u8]) -> [u8; 64] {
    let key = SigningKey::from_bytes(secret32);
    key.sign(message).to_bytes()
}

fn decode_sig(bytes: &[u8]) -> Option<[u8; 64]> {
    if bytes.len() == 64 {
        let mut arr = [0u8; 64];
        arr.copy_from_slice(bytes);
        return Some(arr);
    }
    let text = std::str::from_utf8(bytes).ok()?.trim();
    let decoded = crate::parse_hex(text)?;
    if decoded.len() != 64 {
        return None;
    }
    let mut arr = [0u8; 64];
    arr.copy_from_slice(&decoded);
    Some(arr)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn public_keys_parse() {
        for raw in PUBLIC_KEYS {
            VerifyingKey::from_bytes(&raw).unwrap();
        }
    }

    #[test]
    fn independent_key_roundtrip() {
        let sk = SigningKey::from_bytes(&[7u8; 32]);
        let msg = b"hello";
        let sig = sk.sign(msg);
        assert!(sk.verifying_key().verify(msg, &sig).is_ok());
        assert!(!verify(msg, &sig.to_bytes()));
    }
}
