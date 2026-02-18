use chacha20poly1305::aead::{Aead, OsRng};
use chacha20poly1305::{AeadCore, ChaCha20Poly1305, Key, KeyInit, Nonce};

pub(crate) fn encrypt(string: &String) -> Option<Vec<u8>> {
    let key = ChaCha20Poly1305::generate_key(&mut OsRng);
    let cipher = ChaCha20Poly1305::new(&key);
    let nonce = ChaCha20Poly1305::generate_nonce(&mut OsRng);
    match cipher.encrypt(&nonce, string.as_bytes()) {
        Ok(e) => {
            let mut encrypted = Vec::from(key.as_slice());
            encrypted.extend_from_slice(&nonce);
            encrypted.extend_from_slice(&e);
            let mut output = Vec::from(encrypted.len().to_be_bytes());
            output.extend_from_slice(encrypted.as_slice());
            Some(output)
        }
        Err(_) => None,
    }
}

pub(crate) fn decrypt(bytes: &[u8]) -> Option<String> {
    let key = Key::from_slice(&bytes[0..32]);
    let cipher = ChaCha20Poly1305::new(&key);
    let nonce = Nonce::from_slice(&bytes[32..44]);
    let text = &bytes[44..];
    match cipher.decrypt(&nonce, text) {
        Ok(d) => {
            let decrypted = String::from_utf8(d).unwrap();
            Some(decrypted)
        }
        Err(_) => None,
    }
}
