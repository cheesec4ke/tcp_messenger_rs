use chacha20poly1305::aead::Aead;
use chacha20poly1305::{aead, AeadCore, ChaCha20Poly1305, Key, KeyInit};
use rand_chacha::ChaCha20Rng;

pub(crate) fn encrypt(
    bytes: &[u8],
    key: &[u8; 32],
    csprng: &mut ChaCha20Rng,
    message_num: &u128,
) -> Option<Vec<u8>> {
    let cipher = ChaCha20Poly1305::new(Key::from_slice(key));
    csprng.set_word_pos(*message_num);
    let nonce = ChaCha20Poly1305::generate_nonce(csprng);
    match cipher.encrypt(&nonce, bytes) {
        Ok(e) => {
            let mut output = Vec::from(e.len().to_le_bytes()); //size header
            output.extend_from_slice(&*e);
            Some(output)
        }
        Err(_) => None,
    }
}

pub(crate) fn decrypt(
    bytes: &[u8],
    key: &[u8; 32],
    csprng: &mut ChaCha20Rng,
    message_num: &u128,
) -> Result<Vec<u8>, aead::Error> {
    let cipher = ChaCha20Poly1305::new(Key::from_slice(key));
    csprng.set_word_pos(*message_num);
    let nonce = ChaCha20Poly1305::generate_nonce(csprng);
    match cipher.decrypt(&nonce, bytes) {
        Ok(d) => Ok(d),
        Err(e) => Err(e),
    }
}
