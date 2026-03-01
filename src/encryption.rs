use chacha20poly1305::aead::Aead;
use chacha20poly1305::aead::common::Generate;
use chacha20poly1305::{ChaCha20Poly1305, Key, KeyInit, Nonce};
use color_eyre::Result;
use std::io::{Read, Write};
use std::net::TcpStream;
use x25519_dalek::{EphemeralSecret, PublicKey};

pub(crate) fn encrypt(bytes: &[u8], key: &[u8; 32]) -> Result<Vec<u8>> {
    let cipher = ChaCha20Poly1305::new(&Key::try_from(*key)?);
    let nonce = Nonce::generate();
    let mut encrypted = cipher.encrypt(&nonce, bytes)?;
    let mut output = Vec::from(nonce.0);
    output.append(&mut encrypted);

    Ok(output)
}

pub(crate) fn decrypt(bytes: &[u8], key: &[u8; 32]) -> Result<Vec<u8>> {
    let cipher = ChaCha20Poly1305::new(&Key::try_from(*key)?);
    //get the first 12 bytes as a sized array, needed for conversion
    let n = [
        bytes[0], bytes[1], bytes[2], bytes[3], bytes[4], bytes[5],
        bytes[6], bytes[7], bytes[8], bytes[9], bytes[10], bytes[11]
    ];
    let nonce = Nonce::from(n);

    Ok(cipher.decrypt(&nonce, &bytes[12..])?)
}

pub(crate) fn establish_shared_secret(stream: &mut TcpStream) -> Result<[u8; 32]> {
    let mut buf = [0u8; 32];
    let es = EphemeralSecret::random(); //random_from_rng(OsRng);
    let pk = PublicKey::from(&es);
    stream.write_all(&pk.to_bytes())?;
    stream.flush()?;
    stream.read_exact(&mut buf)?;
    let secret = es.diffie_hellman(&PublicKey::from(buf)).to_bytes();

    Ok(secret)
}
