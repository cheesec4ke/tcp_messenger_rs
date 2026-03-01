use std::io;
use std::io::{Read, Write};
use std::net::TcpStream;
use chacha20poly1305::aead::{Aead, OsRng};
use chacha20poly1305::{aead, AeadCore, ChaCha20Poly1305, Key, KeyInit};
use rand_chacha::ChaCha20Rng;
use rand_chacha::rand_core::SeedableRng;
use x25519_dalek::{EphemeralSecret, PublicKey};

pub(crate) fn encrypt(
    bytes: &[u8],
    key: &[u8; 32],
    message_num: &mut u128,
) -> Result<Vec<u8>, aead::Error> {
    let cipher = ChaCha20Poly1305::new(Key::from_slice(key));
    let mut csprng = ChaCha20Rng::from_seed(*key);
    csprng.set_word_pos(*message_num);
    *message_num += 1;
    let nonce = ChaCha20Poly1305::generate_nonce(csprng);
    cipher.encrypt(&nonce, bytes)
}

pub(crate) fn decrypt(
    bytes: &[u8],
    key: &[u8; 32],
    message_num: &mut u128,
) -> Result<Vec<u8>, aead::Error> {
    let cipher = ChaCha20Poly1305::new(Key::from_slice(key));
    let mut csprng = ChaCha20Rng::from_seed(*key);
    csprng.set_word_pos(*message_num);
    *message_num += 1;
    let nonce = ChaCha20Poly1305::generate_nonce(csprng);
    cipher.decrypt(&nonce, bytes)
}

pub(crate) fn establish_shared_secret(stream: &mut TcpStream) -> io::Result<[u8; 32]> {
    let mut buf = [0u8; 32];
    let es = EphemeralSecret::random_from_rng(OsRng);
    let pk = PublicKey::from(&es);
    stream.write_all(&pk.to_bytes())?;
    stream.flush()?;
    stream.read_exact(&mut buf)?;
    let secret = es.diffie_hellman(&PublicKey::from(buf)).to_bytes();
    Ok(secret)
}