use chacha20poly1305::aead::{Aead, OsRng};
use chacha20poly1305::{AeadCore, ChaCha20Poly1305, Key, KeyInit, Nonce};
use chrono::Local;
use clap::Parser;
use crossterm::style::Color::*;
use crossterm::style::{Color, Print, ResetColor, SetForegroundColor};
use crossterm::terminal::ClearType::CurrentLine;
use crossterm::{cursor, execute, terminal};
use std::fs::{exists, File};
use std::io::{stdin, stdout, BufReader, LineWriter, Read, Write};
use std::net::{Shutdown, TcpListener, TcpStream};
use std::process::exit;
use std::sync::{Arc, Mutex};
use std::thread::{spawn, JoinHandle};
use std::time::{Duration, Instant};

const CONNECTION_TIMEOUT: Duration = Duration::from_secs(10);

#[derive(Parser)]
struct Args {
    #[arg(short = 'p', long, default_value = "0")]
    listen_port: u16,
    #[arg(short = 'i', long, default_value = "127.0.0.1")]
    listen_ip: String,
    #[arg(short, long, default_value = "")]
    nick: String,
    #[arg(long, action, default_value = "false")]
    no_color: bool,
    #[arg(short, long, action, default_value = "false")]
    log_messages: bool,
    #[arg(long, action, default_value = "false")]
    color_logs: bool,
    #[arg(long, default_value = "./messages.log")]
    log_path: String,
    #[arg(long, short, num_args = 0.., value_delimiter = ',')]
    startup_connections: Vec<String>,
}

struct Connections {
    connections: Mutex<Vec<Connection>>,
}
impl Connections {
    fn set_nick(&self, nick: &String) -> std::io::Result<()> {
        for conn in self.connections.lock().unwrap().iter_mut() {
            conn.set_nick(nick)?;
        }
        Ok(())
    }

    fn set_peer_nick(&self, peer_addr: &str, nick: &str) {
        if let Some(index) = self.ip_position(peer_addr) {
            self.connections.lock().unwrap()[index]
                .peer_nick
                .clone_from(&nick.to_string());
        }
    }

    fn send_msg(&self, msg: &String) -> std::io::Result<()> {
        for conn in self.connections.lock().unwrap().iter_mut() {
            conn.send_msg(msg)?;
        }
        Ok(())
    }

    fn disconnect(&self, peer_addr: &str) -> bool {
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

    fn new_connection(
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

struct Connection {
    stream: TcpStream,
    peer_nick: String,
    peer_color: Color,
}
impl Connection {
    fn from_tcp_stream(stream: TcpStream) -> Connection {
        Self {
            peer_nick: stream.peer_addr().unwrap().to_string(),
            stream,
            peer_color: random_color(),
        }
    }

    fn try_clone(&self) -> std::io::Result<Connection> {
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

    fn set_nick(&mut self, nick: &String) -> std::io::Result<()> {
        if !nick.is_empty() {
            let msg = format!("/nick {nick}\n");
            self.send_msg(&msg)?;
        }
        Ok(())
    }
}

fn main() -> std::io::Result<()> {
    let args = Arc::new(Args::parse());
    let nick = Arc::new(Mutex::new(args.nick.clone()));
    let stdin = stdin();
    let mut stdout = stdout();
    let mut input = String::new();
    let connections: Arc<Connections> = Arc::new(Connections {
        connections: Mutex::new(Vec::new()),
    });
    let mut listen_addr = format!("{}:{}", args.listen_ip, args.listen_port);

    if args.log_messages && !exists(&args.log_path)? {
        File::create_new(&args.log_path)?;
    }

    execute!(stdout, cursor::EnableBlinking, ResetColor)?;

    let listener = TcpListener::bind(&listen_addr)?;
    listen_addr = listener.local_addr()?.to_string();
    spawn_listener(listener, args.clone(), connections.clone(), nick.clone());

    connections.new_connection(
        connections.clone(),
        &listen_addr,
        &args.nick,
        args.clone(),
        true,
    )?;
    for connection in &args.startup_connections {
        connections.new_connection(
            connections.clone(),
            connection,
            &nick.lock().unwrap(),
            args.clone(),
            false,
        )?;
    }

    loop {
        input.clear();
        stdin.read_line(&mut input)?;
        execute!(
            stdout,
            cursor::MoveToPreviousLine(1),
            terminal::Clear(CurrentLine),
            Print("  input  : ")
        )?;
        match input.as_str() {
            "/exit\n" | "/x\n" => {
                exit(0);
            }
            _ if input.starts_with("/connect ") || input.starts_with("/c ") => {
                let addr = input.split_whitespace().last().unwrap();
                connections.new_connection(
                    connections.clone(),
                    addr,
                    &nick.lock().unwrap(),
                    args.clone(),
                    false,
                )?;
            }
            _ if input.starts_with("/disconnect ") || input.starts_with("/dc ") => {
                let addr = input.split_whitespace().last().unwrap();
                let disconnected = connections.disconnect(addr);
                if disconnected {
                    //let msg = format!("Disconnected from {addr}");
                    //print_with_time(&msg, DarkGrey, &args.no_color)?;
                } else {
                    let msg = format!("Not connected to {addr}");
                    print_with_time(&msg, Red, &args.no_color)?;
                }
            }
            _ if input.starts_with("/listen ") => {
                let addr = input.split_whitespace().last().unwrap();
                if let Ok(listener) = TcpListener::bind(&addr) {
                    spawn_listener(listener, args.clone(), connections.clone(), nick.clone());
                } else {
                    print_with_time(&format!("Failed to bind to {addr}"), Red, &args.no_color)?;
                }
            }
            _ if input.starts_with("/list_peers") || input.starts_with("/lp") => {
                let conns = connections.connections.lock().unwrap();
                if conns.len() == 0 {
                    print_with_time("No peers connected", Red, &args.no_color)?;
                }
                for conn in conns.iter() {
                    let time = Local::now().format("%H:%M:%S");
                    let peer_addr = conn.stream.peer_addr()?.to_string();
                    let has_nick = peer_addr != conn.peer_nick;
                    let peer_nick = if has_nick {
                        conn.peer_nick.clone()
                    } else {
                        String::from("")
                    };
                    if !args.no_color {
                        execute!(
                            stdout,
                            cursor::MoveToColumn(0),
                            SetForegroundColor(DarkGrey),
                            Print(&time),
                            ResetColor,
                            Print(" | "),
                            SetForegroundColor(conn.peer_color),
                            Print(&peer_addr),
                            ResetColor,
                            Print(if has_nick { " (" } else { "" }),
                            SetForegroundColor(conn.peer_color),
                            Print(&peer_nick),
                            ResetColor,
                            Print(if has_nick { ")" } else { "" }),
                            Print("\n  input  : "),
                        )?;
                    } else {
                        print!(
                            "\r{time} | {}{}{}{}\n  input  : ",
                            &peer_addr,
                            if has_nick { " (" } else { "" },
                            peer_nick,
                            if has_nick { ")" } else { "" },
                        );
                        stdout.flush()?;
                    }
                }
            }
            _ if input.starts_with("/nick ") || input.starts_with("/n ") => {
                *nick.lock().unwrap() = input.split_whitespace().last().unwrap().to_string();
                connections.set_nick(&nick.lock().unwrap())?;
            }
            _ => {
                connections.send_msg(&input)?;
            }
        }
    }
}

fn spawn_listener(
    listener: TcpListener,
    args: Arc<Args>,
    connections: Arc<Connections>,
    nick: Arc<Mutex<String>>,
) -> JoinHandle<std::io::Result<()>> {
    spawn(move || -> std::io::Result<()> {
        let addr = listener.local_addr()?;
        print_with_time(&format!("Listening on {addr}..."), DarkGrey, &args.no_color)?;
        for stream in listener.incoming() {
            let s = stream?;
            let mut conn = Connection::from_tcp_stream(s);
            conn.set_nick(&nick.lock().unwrap())?;
            let a = args.clone();
            let c = connections.clone();
            c.connections.lock().unwrap().push(conn.try_clone()?);
            spawn(move || {
                handle_incoming(conn, a, c).unwrap();
            });
        }
        Ok(())
    })
}

fn handle_incoming(
    conn: Connection,
    args: Arc<Args>,
    connections: Arc<Connections>,
) -> std::io::Result<()> {
    let mut stdout = stdout();
    let peer_addr = conn.stream.peer_addr()?.to_string();
    let mut nick = conn.peer_nick;
    let mut reader = BufReader::new(conn.stream);
    let mut msg_len: [u8; 8];
    let mut line: String;
    let mut buf = Vec::new();
    let color = conn.peer_color;
    let mut time = Local::now().format("%H:%M:%S");
    let mut log: Option<LineWriter<File>> = if args.log_messages {
        Some(LineWriter::new(
            File::options()
                .write(true)
                .append(true)
                .open(&*args.log_path)?,
        ))
    } else {
        None
    };

    if !args.no_color {
        execute!(
            stdout,
            cursor::MoveToColumn(0),
            SetForegroundColor(DarkGrey),
            Print(&time),
            ResetColor,
            Print(" | "),
            SetForegroundColor(DarkGrey),
            Print("Accepted connection from"),
            ResetColor,
            Print(" <"),
            SetForegroundColor(color),
            Print(&nick),
            ResetColor,
            Print(">\n  input  : "),
        )?;
    } else {
        print!("\r{time} | Accepted connection from <{nick}>\n  input  : ");
        stdout.flush()?;
    }
    if let Some(log) = &mut log {
        if args.color_logs {
            execute!(
                log,
                SetForegroundColor(DarkGrey),
                Print(&time),
                ResetColor,
                Print(" | <"),
                SetForegroundColor(color),
                Print(&nick),
                ResetColor,
                Print("> "),
                SetForegroundColor(DarkGrey),
                Print("joined\n"),
                ResetColor,
            )?;
        } else {
            log.write(format!("{time} | <{nick}> joined\n").as_bytes())?;
        }
    }

    loop {
        msg_len = [0u8; 8];
        if let Err(_) = reader.read_exact(&mut msg_len) {
            connections.disconnect(&peer_addr);
            if !args.no_color {
                execute!(
                    stdout,
                    cursor::MoveToColumn(0),
                    SetForegroundColor(DarkGrey),
                    Print(&time),
                    ResetColor,
                    Print(" | <"),
                    SetForegroundColor(color),
                    Print(&nick),
                    ResetColor,
                    Print("> "),
                    SetForegroundColor(DarkGrey),
                    Print("disconnected"),
                    ResetColor,
                    Print("\n  input  : "),
                )?;
            } else {
                print!("\r{time} | <{nick}> disconnected\n  input  : ");
                stdout.flush()?;
            }
            if let Some(log) = &mut log {
                if args.color_logs {
                    execute!(
                        log,
                        SetForegroundColor(DarkGrey),
                        Print(&time),
                        ResetColor,
                        Print(" | <"),
                        SetForegroundColor(color),
                        Print(&nick),
                        ResetColor,
                        Print("> "),
                        SetForegroundColor(DarkGrey),
                        Print("disconnected\n"),
                        ResetColor,
                    )?;
                } else {
                    log.write(format!("{time} | <{nick}> disconnected\n").as_bytes())?;
                }
            }

            return Ok(());
        }
        buf.clear();
        buf.resize(u64::from_be_bytes(msg_len) as usize, 0);
        reader.read_exact(&mut buf)?;
        line = decrypt(&buf).unwrap();
        line = line.trim().to_string();
        time = Local::now().format("%H:%M:%S");
        match line {
            _ if line.starts_with("/nick ") || line.starts_with("/n ") => {
                let new_nick = line.split_whitespace().last().unwrap().to_string();
                if !args.no_color {
                    execute!(
                        stdout,
                        cursor::MoveToColumn(0),
                        SetForegroundColor(DarkGrey),
                        Print(&time),
                        ResetColor,
                        Print(" | <"),
                        SetForegroundColor(color),
                        Print(&nick),
                        ResetColor,
                        Print("> "),
                        SetForegroundColor(DarkGrey),
                        Print("changed nickname to"),
                        ResetColor,
                        Print(" <"),
                        SetForegroundColor(color),
                        Print(&new_nick),
                        ResetColor,
                        Print(">\n  input  : "),
                    )?;
                } else {
                    print!("\r{time} | <{nick}> changed nickname to <{new_nick}>\n  input  : ");
                    stdout.flush()?;
                }
                if let Some(log) = &mut log {
                    if args.color_logs {
                        execute!(
                            log,
                            SetForegroundColor(DarkGrey),
                            Print(&time),
                            ResetColor,
                            Print(" | <"),
                            SetForegroundColor(color),
                            Print(&nick),
                            ResetColor,
                            Print("> "),
                            SetForegroundColor(DarkGrey),
                            Print("changed nickname to"),
                            ResetColor,
                            Print(" <"),
                            SetForegroundColor(color),
                            Print(&new_nick),
                            ResetColor,
                            Print(">\n"),
                        )?;
                    } else {
                        log.write(
                            format!("{} | <{}> changed nickname to <{}>\n", time, nick, new_nick)
                                .as_bytes(),
                        )?;
                    }
                }
                nick = new_nick;
                connections.set_peer_nick(&peer_addr, &nick);
            }
            _ => {
                if !args.no_color {
                    execute!(
                        stdout,
                        cursor::MoveToColumn(0),
                        SetForegroundColor(DarkGrey),
                        Print(&time),
                        ResetColor,
                        Print(" | <"),
                        SetForegroundColor(color),
                        Print(&nick),
                        ResetColor,
                        Print("> "),
                        Print(&line),
                        Print("\n  input  : ")
                    )?;
                } else {
                    print!("\r{time} | <{nick}> {line}\n  input  : ");
                    stdout.flush()?;
                }
                if let Some(log) = &mut log {
                    if args.color_logs {
                        execute!(
                            log,
                            SetForegroundColor(DarkGrey),
                            Print(&time),
                            ResetColor,
                            Print(" | <"),
                            SetForegroundColor(color),
                            Print(&nick),
                            ResetColor,
                            Print("> "),
                            Print(&line),
                            Print("\n")
                        )?;
                    } else {
                        log.write(format!("{time} | <{nick}> {line}\n").as_bytes())?;
                    }
                }
            }
        }
    }
}

fn print_with_time(msg: &str, color: Color, monochrome: &bool) -> std::io::Result<()> {
    let mut stdout = stdout();
    let time = Local::now().format("%H:%M:%S");
    if !monochrome {
        execute!(
            stdout,
            cursor::MoveToColumn(0),
            SetForegroundColor(DarkGrey),
            Print(time),
            ResetColor,
            Print(" | "),
            SetForegroundColor(color),
            Print(msg),
            ResetColor,
            Print("\n  input  : ")
        )?;
    } else {
        print!("\r{} | {}\n  input  : ", time, msg);
        stdout.flush()?;
    }
    Ok(())
}

fn random_color() -> Color {
    let colors = [
        Red,
        Green,
        Yellow,
        Blue,
        Magenta,
        Cyan,
        DarkRed,
        DarkGreen,
        DarkYellow,
        DarkBlue,
        DarkMagenta,
        DarkCyan,
    ];
    colors[fastrand::usize(0..12)]
}

fn encrypt(string: &String) -> Option<Vec<u8>> {
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

fn decrypt(bytes: &[u8]) -> Option<String> {
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
