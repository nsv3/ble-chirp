use anyhow::Result;
use chacha20poly1305::{
    ChaCha20Poly1305, Key, Nonce,
    aead::{Aead, KeyInit},
};
use sha2::{Digest, Sha256};

pub type KeyBytes = [u8; 32];

pub fn derive_key(passphrase: &str) -> KeyBytes {
    let digest = Sha256::digest(passphrase.as_bytes());
    let mut k = [0u8; 32];
    k.copy_from_slice(&digest);
    k
}

pub fn encrypt(key: &KeyBytes, msg_id: &[u8; 4], seq: u8, payload: &[u8]) -> Result<Vec<u8>> {
    let cipher = ChaCha20Poly1305::new(Key::from_slice(key));
    let mut nonce_bytes = [0u8; 12];
    nonce_bytes[..4].copy_from_slice(msg_id);
    nonce_bytes[4] = seq;
    let nonce = Nonce::from_slice(&nonce_bytes);
    Ok(cipher.encrypt(nonce, payload)?)
}

pub fn decrypt(key: &KeyBytes, msg_id: &[u8; 4], seq: u8, payload: &[u8]) -> Result<Vec<u8>> {
    let cipher = ChaCha20Poly1305::new(Key::from_slice(key));
    let mut nonce_bytes = [0u8; 12];
    nonce_bytes[..4].copy_from_slice(msg_id);
    nonce_bytes[4] = seq;
    let nonce = Nonce::from_slice(&nonce_bytes);
    Ok(cipher.decrypt(nonce, payload)?)
}
