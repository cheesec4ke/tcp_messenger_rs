mod connections;
mod encryption;

use crate::connections::{Connection, State};
use crate::encryption::{decrypt, encrypt, establish_shared_secret};
use chacha20poly1305::aead::OsRng;
use chrono::{Local, Month};
use clap::Parser;
use crossterm::style::Color::*;
use crossterm::style::{Color, Print, ResetColor, SetForegroundColor};
use crossterm::terminal::ClearType::CurrentLine;
use crossterm::{cursor, execute, queue, terminal, QueueableCommand};
use rand_chacha::rand_core::RngCore;
use std::fs::{exists, File};
use std::io;
use std::io::{stdin, stdout, BufReader, BufWriter, LineWriter, Read, Write};
use std::net::{TcpListener, TcpStream};
use std::path::Path;
use std::process::exit;
use std::sync::{Arc, Mutex, RwLock};
use std::thread::{spawn, JoinHandle};

#[derive(Parser)]
struct Args {
    #[arg(short = 'p', long, default_value = "0")]
    listen_port: u16,
    #[arg(short = 'i', long, default_value = "127.0.0.1")]
    listen_ip: String,
    #[arg(short, long)]
    nick: Option<String>,
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

fn main() -> io::Result<()> {
    let stdin = stdin();
    let mut stdout = stdout();
    let mut input = String::new();
    let args = Arc::new(Args::parse());
    let state: Arc<State> = Arc::new(State {
        connections: RwLock::new(Vec::new()),
        nick: RwLock::new(args.nick.clone()),
    });
    if args.log_messages && !exists(&args.log_path)? {
        File::create_new(&args.log_path)?;
    }

    execute!(stdout, cursor::EnableBlinking, ResetColor)?;

    let listen_addr = format!("{}:{}", args.listen_ip, args.listen_port);
    spawn_listener(listen_addr, args.clone(), state.clone());

    for addr in &args.startup_connections {
        let _local_addr = State::new_connection(
            state.clone(),
            addr,
            args.clone()
        );
    }

    loop {
        input.clear();
        stdin.read_line(&mut input)?;
        /*execute!(
            stdout,
            cursor::MoveToPreviousLine(1),
            terminal::Clear(CurrentLine),
            Print("  input  : ")
        )?;*/
        match input.as_str() {
            "/exit\n" | "/x\n" => exit(0),
            _ if input.starts_with("/connect ") || input.starts_with("/c ") => {
                let addr = input.split_whitespace().last().unwrap();
                let _local_addr = State::new_connection(
                    state.clone(),
                    addr,
                    args.clone(),
                );
            }
            _ if input.starts_with("/disconnect ") || input.starts_with("/dc ") => {
                let addr = input.split_whitespace().last().unwrap();
                let disconnected = state.disconnect(addr, true);
                if disconnected {
                    //let msg = format!("Disconnected from {addr}");
                    //print_with_time(&msg, DarkGrey, &args.no_color)?;
                } else {
                    let msg = format!("Not connected to {addr}");
                    print_with_time(&msg, Red, &args.no_color)?;
                }
            }
            _ if input.starts_with("/listen ") => {
                let addr = input.split_whitespace().last().unwrap().to_string();
                spawn_listener(addr, args.clone(), state.clone());
            }
            "/list_peers\n" | "/lp\n" => {
                list_peers(state.clone(), args.clone())?;
            }
            _ if input.starts_with("/send_file ") || input.starts_with("/sf ") => {
                let input = input.split_whitespace().last().unwrap().to_string();
                let path = Path::new(&input);
                if path.try_exists()? {
                    state.send_file(&path)?;
                }
            }
            _ if input.starts_with("/nick ") || input.starts_with("/n ") => {
                state.set_nick(input.split_whitespace().last().unwrap())?;
            }
            _ => {
                state.send_msg(&input)?;
            }
        }
    }
}

fn list_peers(connections: Arc<State>, args: Arc<Args>) -> io::Result<()> {
    let mut stdout = stdout();
    let conns = connections.connections.read().unwrap();
    if conns.len() == 0 {
        print_with_time("No peers connected", Red, &args.no_color)?;
    }
    for conn in conns.iter() {
        let peer_addr = &conn.peer_addr;
        let peer_nick = &conn.peer_nick.read().unwrap();
        let msg = vec![
            
        ];
        write_msg(&mut stdout, &msg, &args.no_color, true)?;
        /*if !args.no_color {
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
        }*/
    }
    Ok(())
}

fn spawn_listener(
    listen_addr: String,
    args: Arc<Args>,
    state: Arc<State>,
) -> JoinHandle<io::Result<()>> {
    spawn(move || -> io::Result<()> {
        if let Ok(listener) = TcpListener::bind(&listen_addr) {
            let addr = listener.local_addr()?;
            print_with_time(&format!("Listening on {addr}..."), DarkGrey, &args.no_color)?;
            for stream in listener.incoming() {
                if let Ok(stream) = stream {
                    let a = args.clone();
                    let c = state.clone();
                    spawn(move || -> io::Result<()> {
                        handle_connection(stream, a, c, true)
                    });
                }
            }
        } else {
            print_with_time(&format!("Failed to bind to {listen_addr}"), Red, &args.no_color)?;
        }
        Ok(())
    })
}

fn handle_connection(
    stream: TcpStream,
    args: Arc<Args>,
    state: Arc<State>,
    incoming: bool
) -> io::Result<()> {
    let mut stdout = stdout();
    let mut reader = BufReader::new(stream.try_clone()?);
    let mut header = [0u8; 8];
    let mut line: String;
    let mut buf = Vec::new();
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

    let mut connection = Connection::new(stream)?;
    
    let mut peer_color = connection.peer_color.read().unwrap().clone();
    let peer_addr = connection.peer_addr;
    connection.update_nick(&state.nick.read().unwrap())?;
    state.connections.write().unwrap().push(connection);

    let addr_position = state.addr_position(&peer_addr).unwrap();
    let mut conns = state.connections.read().unwrap();
    let mut peer_nick = conns[addr_position].peer_nick.read().unwrap().clone();
    let color = conns[addr_position].peer_color.read().unwrap().clone();
    drop(conns);

    if incoming {
        let msg = vec![
            ("<", &Reset),
            (&peer_addr, &peer_color),
            ("> ", &Reset),
            ("connected", &DarkGrey),
        ];
        write_msg(&mut stdout, &msg, &args.no_color, true)?;
        if let Some(mut log) = log {
            write_msg(&mut log, &msg, &args.no_color, false)?;
        }
    } else {
        let msg = vec![
            ("connected to", &DarkGrey),
            (" <", &Reset),
            (&peer_addr, &peer_color),
            (">", &Reset),
        ];
        write_msg(&mut stdout, &msg, &args.no_color, true)?;
        if let Some(mut log) = log {
            write_msg(&mut log, &msg, &args.no_color, false)?;
        }
    }

    loop {
        let n = if let Some(n) = state.nick.read().unwrap().clone() {
            n
        } else {
            state.connections
                .read()
                .unwrap()[state.addr_position(&peer_addr).unwrap()]
                .peer_addr
                .clone()
        };
        if let Err(_) = reader.read_exact(&mut header) {
            state.disconnect(&peer_addr, true);

            let msg = vec![
                ("<", Reset),
                
            ];
            if !args.no_color {
                execute!(
                    stdout,
                    cursor::MoveToColumn(0),
                    SetForegroundColor(DarkGrey),
                    Print(&time),
                    ResetColor,
                    Print(" | <"),
                    SetForegroundColor(color),
                    Print(&peer_nick),
                    ResetColor,
                    Print("> "),
                    SetForegroundColor(DarkGrey),
                    Print("disconnected"),
                    ResetColor,
                    Print("\n  input  : "),
                )?;
            } else {
                print!("\r{time} | <{peer_nick}> disconnected\n  input  : ");
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
                        Print(&peer_nick),
                        ResetColor,
                        Print("> "),
                        SetForegroundColor(DarkGrey),
                        Print("disconnected\n"),
                        ResetColor,
                    )?;
                } else {
                    log.write(format!("{time} | <{peer_nick}> disconnected\n").as_bytes())?;
                }
            }
            return Ok(());
        }
        let info = header[7];
        header[7] = 0;
        if info == 255 {
            let conns = state.connections.read().unwrap();
            drop(conns);
            let file_size = u64::from_le_bytes(header);
            reader.read_exact(&mut header)?;
            buf.resize(u64::from_le_bytes(header) as usize, 0);
            reader.read_exact(&mut buf)?;
            let file_name =
                String::from_utf8(decrypt(
                    &buf, 
                    &secret,
                    &mut csprng, 
                    &messages_sent
                ).unwrap()).unwrap();
            let path = Path::new(&file_name);
            let msg = format!("Receiving file \"{file_name}\" ({file_size} bytes)");
            print_with_time(&msg, DarkYellow, &args.no_color)?;
            let file = if let Ok(f) = File::create_new(&path) {
                Some(f)
            } else {
                let mut fl = None;
                'rename: for n in 1..100 {
                    let new_path = format!(
                        "{}_{}{}",
                        path.file_prefix().unwrap().to_str().unwrap(),
                        n,
                        if let Some(e) = path.extension() {
                            let mut s = String::from('.');
                            s.push_str(e.to_str().unwrap());
                            s
                        } else {
                            String::new()
                        }
                    );
                    if let Ok(f) = File::create_new(new_path) {
                        fl = Some(f);
                        break 'rename;
                    }
                }
                fl
            };
            if let Some(mut f) = file {
                let mut buf_writer = BufWriter::new(&mut f);
                let pieces = (file_size + 8000u64 - 1) / 8000u64;
                for p in 0..pieces {
                    reader.read_exact(&mut header)?;
                    buf.resize(u64::from_le_bytes(header) as usize, 0);
                    reader.read_exact(&mut buf)?;
                    let bytes = decrypt(&buf, &secret, &mut csprng, &messages_sent).unwrap();
                    buf_writer.write_all(&bytes)?;
                    buf_writer.flush()?;
                    let msg = format!("Received piece {}/{pieces}", p + 1);
                    print_with_time(&msg, DarkGrey, &args.no_color)?;
                }
            } else {
                //todo failed to create file
            }
        } else {
            buf.resize(u64::from_le_bytes(header) as usize, 0);
            reader.read_exact(&mut buf)?;
            /*unsafe {
                print_with_time(str::from_utf8_unchecked(buf.as_slice()), DarkGrey, &args.no_color)?
            };*/
            let addr_position = state.addr_position(&peer_addr).unwrap();
            let conns = state.connections.read().unwrap();
            let conn = &conns[addr_position];
            line = String::from_utf8(
                decrypt(
                    &buf,
                    &conn.secret,
                    &mut conn.message_num.lock().unwrap(),
                ).unwrap(),
            )
            .unwrap();
            drop(conns);
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
                            Print(&peer_nick),
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
                        print!("\r{time} | <{peer_nick}> changed nickname to <{new_nick}>\n  input  : ");
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
                                Print(&peer_nick),
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
                                format!(
                                    "{} | <{}> changed nickname to <{}>\n",
                                    time, peer_nick, new_nick
                                ).as_bytes(),
                            )?;
                        }
                    }
                    peer_nick = new_nick;
                    state.set_peer_nick(&peer_addr, &peer_nick);
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
                            Print(&peer_nick),
                            ResetColor,
                            Print("> "),
                            Print(&line),
                            Print("\n  input  : ")
                        )?;
                    } else {
                        print!("\r{time} | <{peer_nick}> {line}\n  input  : ");
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
                                Print(&peer_nick),
                                ResetColor,
                                Print("> "),
                                Print(&line),
                                Print("\n")
                            )?;
                        } else {
                            log.write(format!("{time} | <{peer_nick}> {line}\n").as_bytes())?;
                        }
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

fn write_msg(
    writer: &mut impl Write, 
    msg: &Vec<(&str, &Color)>, 
    monochrome: &bool,
    stdout: bool,
) -> io::Result<()> {
    let time = Local::now().format("%H:%M:%S");
    if !monochrome {
        queue!(
            writer,
            cursor::MoveToColumn(0),
            SetForegroundColor(DarkGrey),
            Print(time),
            ResetColor,
            Print(" | "),
        )?;
        for m in msg {
            queue!(
                writer,
                SetForegroundColor(*m.1),
                Print(m.0),
            )?;
        }
        queue!(
            writer,
            ResetColor,
            Print("\n"),
        )?;
        if stdout {
            queue!(
                writer,
                Print("  input  : ")
            )?;
        }
    } else {
        write!(writer, "\r{time} | ")?;
        for m in msg {
            write!(writer, "{}", m.0)?;
        }
        write!(writer, "\n")?;
        if stdout {
            write!(writer, "  input  : ")?;
        }
    }
    writer.flush()?;
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
    colors[OsRng.next_u32() as usize % 12]
}
