use crate::{encrypt, handle_incoming, print_with_time, random_color, Args};
use crossterm::style::Color;
use crossterm::style::Color::{DarkGrey, Red};
use std::io::Write;
use std::net::{Shutdown, TcpStream};
use std::sync::{Arc, Mutex};
use std::thread::spawn;
use std::time::{Duration, Instant};

const CONNECTION_TIMEOUT: Duration = Duration::from_secs(10);

pub(crate) struct Connections {
    pub(crate) connections: Mutex<Vec<Connection>>,
}
impl Connections {
    pub(crate) fn set_nick(&self, nick: &String) -> std::io::Result<()> {
        for conn in self.connections.lock().unwrap().iter_mut() {
            conn.set_nick(nick)?;
        }
        Ok(())
    }

    pub(crate) fn set_peer_nick(&self, peer_addr: &str, nick: &str) {
        if let Some(index) = self.ip_position(peer_addr) {
            self.connections.lock().unwrap()[index]
                .peer_nick
                .clone_from(&nick.to_string());
        }
    }

    pub(crate) fn send_msg(&self, msg: &String) -> std::io::Result<()> {
        for conn in self.connections.lock().unwrap().iter_mut() {
            conn.send_msg(msg)?;
        }
        Ok(())
    }

    pub(crate) fn disconnect(&self, peer_addr: &str) -> bool {
        let mut disconnected = false;
        self.connections.lock().unwrap().retain_mut(|s| {
            if let Ok(a) = s.stream.peer_addr() {
                if a.to_string() != *peer_addr {
                    true
                } else {
                    s.stream.shutdown(Shutdown::Both).unwrap();
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

    fn ip_position(&self, ip: &str) -> Option<usize> {
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
    ) -> std::io::Result<()> {
        let mut msg: String;
        if self.ip_position(&addr).is_some() {
            msg = format!("Already connected to {addr}");
            print_with_time(&msg, Red, &args.no_color)?;
        } else {
            msg = format!("Connecting to {addr}...");
            print_with_time(&msg, DarkGrey, &args.no_color)?;
            if let Some(stream) = connect(addr) {
                let mut connection = Connection::from_tcp_stream(stream);
                connection.set_nick(&nick)?;
                self.connections.lock().unwrap().push(connection);
                if !send_only {
                    let a = args.clone();
                    let c = self
                        .connections
                        .lock()
                        .unwrap()
                        .last()
                        .unwrap()
                        .try_clone()?;
                    let conns = connections.clone();
                    spawn(|| {
                        handle_incoming(c, a, conns).unwrap();
                    });
                }
                msg = format!("Connected to {addr}");
                print_with_time(&msg, DarkGrey, &args.no_color)?;
            } else {
                msg = format!("Unable to connect to {addr}");
                print_with_time(&msg, Red, &args.no_color)?;
            }
        }

        return Ok(());

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

pub(crate) struct Connection {
    pub(crate) stream: TcpStream,
    pub(crate) peer_nick: String,
    pub(crate) peer_color: Color,
}
impl Connection {
    pub(crate) fn from_tcp_stream(stream: TcpStream) -> Connection {
        Self {
            peer_nick: stream.peer_addr().unwrap().to_string(),
            stream,
            peer_color: random_color(),
        }
    }

    pub(crate) fn try_clone(&self) -> std::io::Result<Connection> {
        match self.stream.try_clone() {
            Ok(stream) => Ok(Self {
                stream,
                peer_nick: self.peer_nick.clone(),
                peer_color: self.peer_color.clone(),
            }),
            Err(e) => Err(e),
        }
    }

    fn send_msg(&mut self, msg: &String) -> std::io::Result<()> {
        let encrypted = encrypt(msg).unwrap();
        self.stream.write_all(encrypted.as_slice())?;
        self.stream.flush()?;
        Ok(())
    }

    /*fn send_bytes(&mut self, bytes: &[u8]) -> std::io::Result<()> {
        self.stream.write_all(bytes)?;
        self.stream.flush()?;
        Ok(())
    }*/

    pub(crate) fn set_nick(&mut self, nick: &String) -> std::io::Result<()> {
        if !nick.is_empty() {
            let msg = format!("/nick {nick}\n");
            self.send_msg(&msg)?;
        }
        Ok(())
    }
}
