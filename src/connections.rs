use crate::{encrypt, handle_incoming, print_with_time, random_color, Args};
use chacha20poly1305::aead::OsRng;
use crossterm::style::Color;
use crossterm::style::Color::{DarkGrey, Red};
use rand_chacha::rand_core::SeedableRng;
use rand_chacha::ChaCha20Rng;
use std::io;
use std::io::{ErrorKind, Read, Write};
use std::net::{Shutdown, TcpStream};
use std::sync::{Arc, Mutex};
use std::thread::spawn;
use std::time::{Duration, Instant};
use x25519_dalek::{EphemeralSecret, PublicKey};

const CONNECTION_TIMEOUT: Duration = Duration::from_secs(10);

#[derive(Debug)]
pub(crate) struct Connections {
    pub(crate) connections: Mutex<Vec<Connection>>,
}
impl Connections {
    pub(crate) fn set_nick(&self, nick: &String) -> io::Result<()> {
        for conn in self.connections.lock().unwrap().iter_mut() {
            conn.set_nick(nick)?;
        }
        Ok(())
    }

    pub(crate) fn set_peer_nick(&self, peer_addr: &str, nick: &str) {
        if let Some(index) = self.addr_position(peer_addr) {
            self.connections.lock().unwrap()[index]
                .peer_nick
                .clone_from(&nick.to_string());
        }
    }

    pub(crate) fn send_msg(&self, msg: &String) -> io::Result<()> {
        for conn in self.connections.lock().unwrap().iter_mut() {
            conn.send_msg(msg)?;
        }
        Ok(())
    }

    pub(crate) fn disconnect(&self, peer_addr: &str, shutdown: bool) -> bool {
        let mut disconnected = false;
        self.connections.lock().unwrap().retain_mut(|s| {
            if let Ok(a) = s.stream.peer_addr() {
                if a.to_string() != *peer_addr {
                    true
                } else {
                    if shutdown {
                        s.stream.shutdown(Shutdown::Both).unwrap();
                    }
                    disconnected = true;
                    false
                }
            } else {
                disconnected = true;
                false
            }
        });
        disconnected
    }

    /*fn get_addr(&self, nick: &String) -> Option<String> {
        if let Some(index) = self.nick_position(nick) {
            Some(
                self.connections.lock().unwrap()[index]
                    .stream
                    .peer_addr()
                    .unwrap()
                    .to_string(),
            )
        } else {
            None
        }
    }*/

    pub(crate) fn addr_position(&self, ip: &str) -> Option<usize> {
        self.connections
            .lock()
            .unwrap()
            .iter()
            .position(|c| c.stream.peer_addr().unwrap().to_string().eq(ip))
    }

    /*fn nick_position(&self, nick: &str) -> Option<usize> {
        self.connections
            .lock()
            .unwrap()
            .iter()
            .position(|c| c.peer_nick.eq(nick))
    }*/

    pub(crate) fn new_connection(
        &self,
        connections: Arc<Self>,
        addr: &str,
        nick: &String,
        args: Arc<Args>,
        send_only: bool,
        local: bool,
    ) -> io::Result<String> {
        let mut msg: String;
        let mut local_addr = None;
        if self.addr_position(&addr).is_some() {
            msg = format!("Already connected to {addr}");
            print_with_time(&msg, Red, &args.no_color)?;
        } else {
            msg = format!("Connecting to {addr}...");
            print_with_time(&msg, DarkGrey, &args.no_color)?;
            if let Some(stream) = connect(addr) {
                let p = stream.peer_addr()?.to_string();
                local_addr = Some(stream.local_addr()?.to_string());
                let mut conn = Connection::from_tcp_stream(stream);
                conn.local = local;
                let es = EphemeralSecret::random_from_rng(OsRng);
                let pk = PublicKey::from(&es);
                conn.send_bytes(pk.as_bytes())?;
                let mut buf = [0u8; 32];
                conn.stream.read_exact(&mut buf)?;
                conn.secret = es.diffie_hellman(&PublicKey::from(buf)).to_bytes();
                conn.csprng = ChaCha20Rng::from_seed(conn.secret);
                conn.csprng.set_word_pos(0);
                conn.set_nick(&nick)?;
                self.connections.lock().unwrap().push(conn);
                if !send_only {
                    let a = args.clone();
                    let conns = connections.clone();
                    spawn(move || {
                        handle_incoming(&p, a, conns).unwrap();
                    });
                }
                msg = format!("Connected to {addr}");
                print_with_time(&msg, DarkGrey, &args.no_color)?;
            } else {
                msg = format!("Unable to connect to {addr}");
                print_with_time(&msg, Red, &args.no_color)?;
            }
        }

        return local_addr.ok_or(io::Error::new(ErrorKind::Other, "no connection available"));

        fn connect(addr: &str) -> Option<TcpStream> {
            let now = Instant::now();
            loop {
                if let Ok(stream) = TcpStream::connect(addr) {
                    return Some(stream);
                }
                if now.elapsed() > CONNECTION_TIMEOUT {
                    return None;
                }
            }
        }
    }
}

#[derive(Debug)]
pub(crate) struct Connection {
    pub(crate) stream: TcpStream,
    pub(crate) peer_nick: String,
    pub(crate) peer_color: Color,
    pub(crate) secret: [u8; 32],
    pub(crate) csprng: ChaCha20Rng,
    pub(crate) messages_sent: u128,
    pub(crate) local: bool,
}
impl Connection {
    pub(crate) fn from_tcp_stream(stream: TcpStream) -> Connection {
        Self {
            peer_nick: stream.peer_addr().unwrap().to_string(),
            stream,
            peer_color: random_color(),
            secret: [0u8; 32],
            csprng: ChaCha20Rng::from_seed([0u8; 32]),
            messages_sent: 0u128,
            local: false,
        }
    }

    /*pub(crate) fn try_clone(&self) -> std::io::Result<Connection> {
        match self.stream.try_clone() {
            Ok(stream) => Ok(Self {
                stream,
                peer_nick: self.peer_nick.clone(),
                peer_color: self.peer_color.clone(),
            }),
            Err(e) => Err(e),
        }
    }*/

    fn send_msg(&mut self, msg: &String) -> io::Result<()> {
        let encrypted = encrypt(msg, &self.secret, &mut self.csprng, &self.messages_sent).unwrap();
        self.messages_sent += 1;
        self.stream.write_all(encrypted.as_slice())?;
        self.stream.flush()?;
        Ok(())
    }

    pub(crate) fn send_bytes(&mut self, bytes: &[u8]) -> io::Result<()> {
        self.stream.write_all(bytes)?;
        self.stream.flush()?;
        Ok(())
    }

    pub(crate) fn set_nick(&mut self, nick: &String) -> io::Result<()> {
        if !nick.is_empty() {
            let msg = format!("/nick {nick}\n");
            self.send_msg(&msg)?;
        }
        Ok(())
    }
}
