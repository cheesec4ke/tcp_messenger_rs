use crate::{encrypt, handle_connection, print_with_time, random_color, Args};
use crossterm::style::Color;
use crossterm::style::Color::{DarkGrey, Red};
use std::fs::File;
use std::io;
use std::io::{BufReader, BufWriter, ErrorKind, Read, Write};
use std::net::{Shutdown, TcpStream};
use std::path::Path;
use std::sync::{Arc, Mutex, RwLock};
use std::thread::spawn;
use std::time::{Duration, Instant};
use crate::encryption::establish_shared_secret;

const CONNECTION_TIMEOUT: Duration = Duration::from_secs(10);
const PIECE_SIZE: u64 = 8000;

type Connections = RwLock<Vec<Connection>>;
#[derive(Debug)]
pub(crate) struct State {
    pub(crate) nick: RwLock<Option<String>>,
    pub(crate) connections: Connections,
}
impl State {
    pub(crate) fn set_nick(&self, nick: &str) -> io::Result<()> {
        self.nick.write().unwrap().replace(nick.to_owned());
        let n = self.nick.read().unwrap();
        for conn in self.connections.read().unwrap().iter() {
            conn.update_nick(&n)?;
        }
        Ok(())
    }

    pub(crate) fn set_peer_nick(&self, peer_addr: &str, nick: &str) {
        if let Some(index) = self.addr_position(peer_addr) {
            self.connections
                .read()
                .unwrap()[index]
                .peer_nick
                .lock()
                .unwrap()
                .clone_from(&Some(nick.to_string()));
        }
    }

    pub(crate) fn send_msg(&self, msg: &String) -> io::Result<()> {
        for conn in self.connections.write().unwrap().iter_mut() {
            conn.send_msg(msg)?;
        }
        Ok(())
    }

    pub(crate) fn send_file(&self, path: &Path) -> io::Result<()> {
        for conn in self.connections.write().unwrap().iter_mut() {
            conn.send_file(&path)?;
        }
        Ok(())
    }

    pub(crate) fn disconnect(&self, peer_addr: &str, shutdown: bool) -> bool {
        let mut disconnected = false;
        self.connections.write().unwrap().retain_mut(|s| {
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
            .read()
            .unwrap()
            .iter()
            .position(|c| c.peer_addr.eq(ip))
    }

    /*fn nick_position(&self, nick: &str) -> Option<usize> {
        self.connections
            .lock()
            .unwrap()
            .iter()
            .position(|c| c.peer_nick.eq(nick))
    }*/

    pub(crate) fn new_connection(
        state: Arc<Self>,
        addr: &str,
        args: Arc<Args>,
    ) -> io::Result<()> {
        let msg: String;

        if state.addr_position(&addr).is_some() {
            msg = format!("Already connected to {addr}");
            print_with_time(&msg, Red, &args.no_color)?;
        } else {
            msg = format!("Connecting to {addr}...");
            print_with_time(&msg, DarkGrey, &args.no_color)?;
            let now = Instant::now();
            loop {
                if let Ok(s) = TcpStream::connect(addr) {
                    spawn(move || -> io::Result<()> {
                        handle_connection(s, args, state, false)
                    });
                    break;
                }
                if now.elapsed() > CONNECTION_TIMEOUT {
                    return Err(io::Error::new(
                        ErrorKind::Other,
                        format!("Connection to {addr} timed out")
                    ));
                }
            }
        }
        Ok(())
    }
}

#[derive(Debug)]
pub(crate) struct Connection {
    pub(crate) stream: TcpStream,
    pub(crate) local_addr: String,
    pub(crate) peer_addr: String,
    pub(crate) peer_nick: RwLock<Option<String>>,
    pub(crate) peer_color: RwLock<Color>,
    pub(crate) secret: [u8; 32],
    pub(crate) message_num: Mutex<u128>,
}
impl Connection {
    /*pub(crate) fn connect(addr: &str) -> io::Result<Self> {
        let now = Instant::now();
        loop {
            if let Ok(stream) = TcpStream::connect(addr) {
                Self::new(stream)?;
                break Ok(/* value */);
            }
            if now.elapsed() > CONNECTION_TIMEOUT {
                return Err(io::Error::new(
                    ErrorKind::Other,
                    format!("Connection to {addr} timed out")
                ));
            }
        }
    }*/

    pub(crate) fn new(mut stream: TcpStream) -> io::Result<Self> {
        let secret = establish_shared_secret(&mut stream)?;
        let local_addr = stream.local_addr()?.to_string();
        let peer_addr = stream.peer_addr()?.to_string();
        Ok(Connection {
            stream,
            local_addr,
            peer_addr,
            peer_nick: Mutex::new(None),
            peer_color: Mutex::new(random_color()),
            secret,
            message_num: Mutex::new(0),
        })
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

    fn send_msg(&self, msg: &str) -> io::Result<()> {
        let mut message_num = self.message_num.lock().unwrap();
        let mut writer = BufWriter::new(&self.stream);
        let encrypted = encrypt(
            msg.as_bytes(),
            &self.secret,
            &mut message_num,
        ).unwrap();
        let header = header(&encrypted, 0);
        writer.write_all(&header)?;
        writer.write_all(encrypted.as_slice())?;
        writer.flush()?;
        Ok(())
    }

    pub(crate) fn send_file(&self, path: &Path) -> io::Result<bool> {
        match File::open(path) {
            Ok(file) => {
                let mut stream_writer = BufWriter::new(&self.stream);

                let file_size = file.metadata()?.len();
                let mut header = file_size.to_be_bytes().to_vec();
                header[0] = 255;
                stream_writer.write_all(&header)?;
                let name = path.file_name().unwrap().to_str().unwrap();
                let enc_name = encrypt(
                    name.as_bytes(),
                    &self.secret,
                    &mut self.message_num.lock().unwrap(),
                ).unwrap();
                //dbg!((&enc_name, &self.stream, self.secret, self.messages_sent));
                header = enc_name.len().to_be_bytes().to_vec();
                stream_writer.write(&enc_name.as_slice())?;

                let file_reader = &mut BufReader::new(file);
                let mut buffer = Vec::with_capacity(PIECE_SIZE as usize);
                let pieces = (file_size + PIECE_SIZE- 1) / PIECE_SIZE;
                for _piece in 0..pieces {
                    buffer.clear();
                    //let msg = format!("reading piece {}/{pieces}", piece + 1);
                    //print_with_time(&msg, Red, &false)?;
                    file_reader.take(PIECE_SIZE).read_to_end(&mut buffer)?;
                    let e = encrypt(
                        &buffer,
                        &self.secret,
                        &mut self.message_num.lock().unwrap(),
                    ).unwrap();
                    stream_writer.write_all(&e)?;
                }
                stream_writer.flush()?;

                Ok(true)
            }
            Err(e) => Err(e),
        }
    }

    pub(crate) fn send_bytes(&self, bytes: &[u8]) -> io::Result<()> {
        let mut writer = BufWriter::new(&self.stream);
        writer.write_all(bytes)?;
        writer.flush()?;
        Ok(())
    }

    pub(crate) fn update_nick(&self, nick: &Option<String>) -> io::Result<()> {
        if let Some(n) = nick {
            let msg = format!("/nick {n}\n");
            self.send_msg(&msg)?;
        }
        Ok(())
    }
}

///Generates an 8 byte header with the first bytes representing a message type
///and the next 7 bytes being the length of the message in big-endian bytes
///
///Message types:
///
///0: normal message
///
///254: nick change
///
///255: file
///
///Technically fails if the message is more than 256 TiB
///as the 1st byte of the length is ignored
pub(crate) fn header(msg: &Vec<u8>, msg_type: u8) -> [u8; 8] {
    let mut header: [u8; 8] = msg.len().to_be_bytes();
    header[0] = msg_type;
    header
}