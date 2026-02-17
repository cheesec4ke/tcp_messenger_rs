use chrono::Local;
use clap::Parser;
use crossterm::style::Color::*;
use crossterm::style::{Color, Print, ResetColor, SetForegroundColor};
use crossterm::terminal::ClearType::CurrentLine;
use crossterm::{cursor, execute, terminal};
use std::fs::{exists, File};
use std::io::{stdin, stdout, BufRead, BufReader, LineWriter, Write};
use std::net::{TcpListener, TcpStream};
use std::process::exit;
use std::sync::{Arc, Mutex};
use std::thread::spawn;
use std::time::{Duration, Instant};

#[derive(Debug)]
struct Connections {
    connections: Vec<TcpStream>,
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
    color_logs: bool,
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
    let connections: Arc<Mutex<Connections>> = Arc::new(Mutex::new(Connections {
        connections: Vec::new(),
    }));
    let mut msg;
    let addr = format!("{}:{}", args.listen_ip, args.listen_port);
    let addr = addr.as_str();

    if args.log_messages && !exists(&args.log_path)? {
        File::create_new(&args.log_path)?;
    }

    execute!(stdout, cursor::EnableBlinking, ResetColor)?;

    let c = connections.clone();
    let a = args.clone();
    spawn(move || listener(a, c));

    msg = format!("Connecting to {addr}...");
    print_with_time(&msg, DarkGrey, &args.no_color)?;
    if let Some(mut conn) = connect(addr, TIMEOUT) {
        msg = format!("Successfully connected to {addr}");
        print_with_time(&msg, Green, &args.no_color)?;
        if !nick.is_empty() {
            conn.write_all(format!("/nick {nick}\n").as_str().as_bytes())?;
            conn.flush()?;
        }
        connections.lock().unwrap().connections.push(conn);
    } else {
        msg = format!("Unable to connect to {addr}");
        print_with_time(&msg, Red, &args.no_color)?;
    }

    loop {
        input.clear();
        stdin.read_line(&mut input)?;
        match input.as_str() {
            "/exit\n" | "/x\n" => {
                exit(0);
            }
            _ if input.starts_with("/connect ") || input.starts_with("/c ") => {
                execute!(
                    stdout,
                    cursor::MoveToPreviousLine(1),
                    terminal::Clear(CurrentLine)
                )?;
                let addr = input.split_whitespace().last().unwrap();
                if connections
                    .lock()
                    .unwrap()
                    .connections
                    .iter()
                    .find(|c| c.peer_addr().unwrap().to_string() == addr)
                    .is_some()
                {
                    msg = format!("Already connected to {addr}");
                    print_with_time(&msg, Red, &args.no_color)?;
                } else {
                    msg = format!("Connecting to {addr}...");
                    print_with_time(&msg, DarkGrey, &args.no_color)?;
                    if let Some(mut conn) = connect(addr, TIMEOUT) {
                        msg = format!("Successfully connected to {addr}");
                        print_with_time(&msg, Green, &args.no_color)?;
                        if !nick.is_empty() {
                            conn.write_all(format!("/nick {nick}\n").as_str().as_bytes())?;
                            conn.flush()?;
                        }
                        let a = args.clone();
                        let s = conn.try_clone()?;
                        let c = connections.clone();
                        spawn(|| {
                            handle_incoming(s, a, c).unwrap();
                        });
                        connections.lock().unwrap().connections.push(conn);
                    } else {
                        msg = format!("Unable to connect to {addr}");
                        print_with_time(&msg, Red, &args.no_color)?;
                    }
                }
            }
            _ if input.starts_with("/nick ") || input.starts_with("/n ") => {
                nick = input.split_whitespace().last().unwrap().to_string();
                execute!(
                    stdout,
                    cursor::MoveToPreviousLine(1),
                    terminal::Clear(CurrentLine)
                )?;
                for conn in &mut connections.lock().unwrap().connections {
                    conn.write_all(input.as_bytes())?;
                    conn.flush()?;
                }
            }
            _ => {
                execute!(
                    stdout,
                    cursor::MoveToPreviousLine(1),
                    terminal::Clear(CurrentLine)
                )?;
                for conn in &mut connections.lock().unwrap().connections {
                    conn.write_all(input.as_bytes())?;
                    conn.flush()?;
                }
            }
        }
    }
}

fn print_with_time(msg: &String, color: Color, monochrome: &bool) -> std::io::Result<()> {
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

fn listener(args: Arc<Args>, connections: Arc<Mutex<Connections>>) -> std::io::Result<()> {
    let addr = format!("{}:{}", args.listen_ip, args.listen_port);
    let msg = format!("Listening on {addr}");
    print_with_time(&msg, DarkGrey, &args.no_color)?;
    let listener = TcpListener::bind(addr)?;
    for stream in listener.incoming() {
        let mut s = stream?;
        let a = args.clone();
        let c = connections.clone();
        if !args.nick.is_empty() {
            s.write_all(format!("/nick {}\n", args.nick).as_str().as_bytes())?;
            s.flush()?;
        }
        spawn(move || {
            c.lock().unwrap().connections.push(s.try_clone().unwrap());
            handle_incoming(s, a, c).unwrap();
        });
    }
    Ok(())
}

fn handle_incoming(
    stream: TcpStream,
    args: Arc<Args>,
    connections: Arc<Mutex<Connections>>,
) -> std::io::Result<()> {
    let mut stdout = stdout();
    let peer_addr = stream.peer_addr()?.to_string();
    let mut reader = BufReader::new(&stream);
    let mut line = String::new();
    let mut nick = peer_addr.clone();
    let color = random_color();
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
            SetForegroundColor(Green),
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
                SetForegroundColor(Green),
                Print("joined\n"),
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
            connections
                .lock()
                .unwrap()
                .connections
                .retain(|s| s.peer_addr().unwrap() != stream.peer_addr().unwrap());
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
                    Print("disconnected\n")
                )?;
            } else {
                print!("\r{time} | <{nick}> disconnected\n");
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
                        Print("disconnected\n")
                    )?;
                } else {
                    log.write(format!("{time} | <{nick}> disconnected\n").as_bytes())?;
                }
            }

            return Ok(());
        }
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
                        Print(">"),
                        SetForegroundColor(DarkGrey),
                        Print(" changed nickname to "),
                        ResetColor,
                        Print("<"),
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

fn connect(addr: &str, timeout: Duration) -> Option<TcpStream> {
    let now = Instant::now();
    loop {
        if let Ok(stream) = TcpStream::connect(addr) {
            return Some(stream);
        }
        if now.elapsed() > timeout {
            return None;
        }
    }
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

//todo: encrypt messages
/*fn encode(data: &[u8]) -> &[u8] {

    data
}*/
