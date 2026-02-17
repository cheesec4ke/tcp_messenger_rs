use std::fs::{exists, File};
use std::io::{stdin, stdout, BufRead, BufReader, LineWriter, Write};
use std::net::{TcpListener, TcpStream};
use std::process::exit;
use std::sync::{Arc, Mutex};
use std::thread::{spawn};
use std::time::{Duration, Instant};
use chrono::Local;
use clap::Parser;
use crossterm::cursor::{MoveToPreviousLine};
use crossterm::{execute, terminal};
use crossterm::style::{Color, Print, ResetColor, SetForegroundColor};
use crossterm::style::Color::*;
use crossterm::terminal::ClearType::CurrentLine;

struct Connection {
    addr: String,
    stream: LineWriter<TcpStream>,
}

impl Connection {
    fn connect(addr: &str, timeout: Duration) -> Option<Connection> {
        let now = Instant::now();
        loop {
            if let Ok(stream) = TcpStream::connect(addr) {
                return Some(Self {
                    addr: addr.to_string(),
                    stream: LineWriter::new(stream),
                })
            }
            if now.elapsed() > timeout {
                return None;
            }
        }
    }
}

struct Connections {
    connections: Vec<Connection>,
}

#[derive(Parser)]
pub struct Args {
    #[arg(short = 'p', long, default_value = "32767")]
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
    colored_logs: bool,
    #[arg(long, default_value = "./messages.log")]
    log_path: String,
}

fn main() -> std::io::Result<()> {
    const TIMEOUT: Duration = Duration::from_millis(5000);
    let args = Arc::new(Args::parse());
    let mut nick = args.nick.clone();
    let stdin = stdin();
    let mut stdout = stdout();
    let mut input = String::new();
    let connections: Arc<Mutex<Connections>> = Arc::new(Mutex::new(Connections { connections: Vec::new() }));
    let mut msg;
    let addr = format!("{}:{}", args.listen_ip, args.listen_port);
    let addr = addr.as_str();

    if args.log_messages && !exists(&args.log_path)? {
        File::create_new(&args.log_path)?;
    }

    let c = connections.clone();
    spawn(move || listener(args, c));

    msg = format!("Connecting to {addr}...\n");
    execute!(stdout, SetForegroundColor(DarkGrey), Print(&msg))?;
    if let Some(mut conn) = Connection::connect(addr, TIMEOUT) {
        msg = format!("Successfully connected to {addr}\n");
        execute!(stdout, SetForegroundColor(Green), Print(&msg), ResetColor)?;
        if !nick.is_empty() {
            conn.stream.write(format!("/nick {nick}\n").as_str().as_bytes())?;
        }
        connections.lock().unwrap().connections.push(conn);
    } else {
        msg = format!("Unable to connect to {addr}\n");
        execute!(stdout, SetForegroundColor(Red), Print(&msg), ResetColor)?;
    }

    loop {
        input.clear();
        stdin.read_line(&mut input)?;
        match input.as_str() {
            "/exit\n" | "/x\n" => {
                exit(0);
            }
            _ if input.starts_with("/connect ") || input.starts_with("/c ") => {
                execute!(stdout, MoveToPreviousLine(1), terminal::Clear(CurrentLine))?;
                let addr = input.split_whitespace().last().unwrap();
                if connections.lock().unwrap().connections.iter().find(|c|
                    c.addr == addr.to_string()).is_some() {
                    msg = format!("Already connected to {addr}\n");
                    execute!(stdout, SetForegroundColor(Red), Print(&msg), ResetColor)?;
                } else {
                    msg = format!("Connecting to {addr}...\n");
                    execute!(stdout, SetForegroundColor(DarkGrey), Print(&msg))?;
                    if let Some(mut conn) = Connection::connect(addr, TIMEOUT) {
                        msg = format!("Successfully connected to {addr}\n");
                        execute!(stdout, SetForegroundColor(Green), Print(&msg), ResetColor)?;
                        if !nick.is_empty() {
                            conn.stream.write(format!("/nick {nick}\n").as_str().as_bytes())?;
                        }
                        connections.lock().unwrap().connections.push(conn);
                    } else {
                        msg = format!("Unable to connect to {addr}\n");
                        execute!(stdout, SetForegroundColor(Red), Print(&msg), ResetColor)?;
                    }
                }
            }
            _ if input.starts_with("/nick ") || input.starts_with("/n ") => {
                nick = input.split_whitespace().last().unwrap().to_string();
                execute!(stdout, MoveToPreviousLine(1), terminal::Clear(CurrentLine))?;
                for conn in &mut connections.lock().unwrap().connections {
                    conn.stream.write(input.as_bytes())?;
                }
            }
            _ => {
                execute!(stdout, MoveToPreviousLine(1), terminal::Clear(CurrentLine))?;
                for conn in &mut connections.lock().unwrap().connections {
                    conn.stream.write(input.as_bytes())?;
                }
            }
        }
    }
}

fn listener(args: Arc<Args>, connections: Arc<Mutex<Connections>>) -> std::io::Result<()> {
    let addr = format!("{}:{}", args.listen_ip, args.listen_port);
    let msg = format!("Listening on {addr}\n");
    execute!(stdout(), SetForegroundColor(Green), Print(msg), ResetColor)?;
    let listener = TcpListener::bind(addr)?;
    for stream in listener.incoming() {
        let s = stream?;
        let a = args.clone();
        let c = connections.clone();
        spawn( move || {
            c.lock().unwrap().connections.push(
                Connection {
                    addr: s.peer_addr().unwrap().to_string(),
                    stream: LineWriter::new(s.try_clone().unwrap()),
                }
            );
            handle_incoming(s, a).unwrap();
        });
    }
    Ok(())
}

fn handle_incoming(stream: TcpStream, args: Arc<Args>) -> std::io::Result<()> {
    let mut stdout = stdout();
    let peer_addr = stream.peer_addr()?.to_string();
    let mut reader = BufReader::new(&stream);
    let mut line = String::new();
    let mut nick = peer_addr.clone();
    let color = random_color();
    let mut time = Local::now().format("%H:%M:%S");
    let mut log: Option<LineWriter<File>> = if args.log_messages {
        Some(LineWriter::new(File::options().write(true).append(true).open(&*args.log_path)?))
    } else {
        None
    };

    if !args.no_color {
        execute!(
            stdout,
            SetForegroundColor(DarkGrey),
            Print(&time),
            ResetColor,
            Print(" | "),
            SetForegroundColor(Green),
            Print("Accepted connection from"),
            ResetColor,
            Print(" <"),
            SetForegroundColor(color),
            Print(&nick),
            ResetColor,
            Print(">\n")
        )?;
    } else {
        println!("{time} | Accepted connection from <{nick}>");
    }
    if let Some(log) = &mut log {
        if args.colored_logs {
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
                SetForegroundColor(Green),
                Print(" joined\n"),
                ResetColor,
            )?;
        } else {
            log.write(format!("{time} | <{nick}> joined\n").as_bytes())?;
        }
    }

    loop {
        line.clear();
        reader.read_line(&mut line)?;
        if line.is_empty() {
            return Ok(())
        }
        line = line.trim().to_string();
        time = Local::now().format("%H:%M:%S");
        match line {
            _ if line.starts_with("/nick ") || line.starts_with("/n ") => {
                let new_nick = line.split_whitespace().last().unwrap().to_string();
                if !args.no_color {
                    execute!(
                        stdout,
                        SetForegroundColor(DarkGrey),
                        Print(&time),
                        ResetColor,
                        Print(" | <"),
                        SetForegroundColor(color),
                        Print(&nick),
                        ResetColor,
                        Print(">"),
                        SetForegroundColor(DarkGrey),
                        Print(" changed nickname to "),
                        ResetColor,
                        Print("<"),
                        SetForegroundColor(color),
                        Print(&new_nick),
                        ResetColor,
                        Print(">\n"),
                    )?;
                } else {
                    println!("{time} | <{nick}> changed nickname to <{new_nick}>");
                }
                if let Some(log) = &mut log {
                    if args.colored_logs {
                        execute!(
                            log,
                            SetForegroundColor(DarkGrey),
                            Print(&time),
                            ResetColor,
                            Print(" | <"),
                            SetForegroundColor(color),
                            Print(&nick),
                            ResetColor,
                            Print(">"),
                            SetForegroundColor(DarkGrey),
                            Print(" changed nickname to "),
                            ResetColor,
                            Print("<"),
                            SetForegroundColor(color),
                            Print(&new_nick),
                            ResetColor,
                            Print(">\n"),
                        )?;
                    } else {
                        log.write(format!("{time} | <{nick}> changed nickname to <{new_nick}>\n").as_bytes())?;
                    }
                }
                nick = new_nick;
            }
            _ => {
                if !args.no_color {
                    execute!(
                        stdout,
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
                    println!("{time} | <{nick}> {line}")
                }
                if let Some(log) = &mut log {
                    if args.colored_logs {
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

fn random_color() -> Color {
    let colors = [Red, Green, Yellow, Blue, Magenta, Cyan, DarkRed, DarkGreen, DarkYellow, DarkBlue, DarkMagenta, DarkCyan];
    colors[fastrand::usize(0..12)]
}

//todo: encrypt messages
/*fn encode(data: &[u8]) -> &[u8] {

    data
}*/