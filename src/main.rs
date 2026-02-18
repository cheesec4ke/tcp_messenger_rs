mod connections;
mod encryption;

use crate::connections::{Connection, Connections};
use crate::encryption::{decrypt, encrypt};
use chrono::Local;
use clap::Parser;
use crossterm::style::Color::*;
use crossterm::style::{Color, Print, ResetColor, SetForegroundColor};
use crossterm::terminal::ClearType::CurrentLine;
use crossterm::{cursor, execute, terminal};
use std::fs::{exists, File};
use std::io::{stdin, stdout, BufReader, LineWriter, Read, Write};
use std::net::TcpListener;
use std::process::exit;
use std::sync::{Arc, Mutex};
use std::thread::{spawn, JoinHandle};

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
