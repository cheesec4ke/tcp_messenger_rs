use crate::app::Event::*;
use crate::config::Config;
use crate::connections::{
    connection_handler, connection_listener, local_ipv4_addrs, send_file, send_msg,
    Connection, MessageType, CONNECTION_RETRIES,
};
use crate::functions::*;
use crate::types::Nick;
use chrono::Local;
use color_eyre::Result;
use ratatui::buffer::Buffer;
use ratatui::crossterm::event::{KeyCode, KeyModifiers};
use ratatui::layout::{Constraint, Layout, Rect};
use ratatui::prelude::{Line, Widget};
use ratatui::style::{Color, Style};
use ratatui::symbols::merge::MergeStrategy::Fuzzy;
use ratatui::text::Span;
use ratatui::widgets::{Block, Padding, Paragraph, Wrap};
use ratatui::{crossterm, DefaultTerminal, Frame};
use std::io::{BufWriter, Write};
use std::net::{Shutdown, TcpStream};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{Receiver, Sender};
use std::sync::{mpsc, Arc, RwLock};
use std::thread::spawn;
use std::time::Duration;
use std::{fs, thread};

type Connections = Vec<Arc<Connection>>;
type Message = Vec<(String, Style)>;

///Events for updating the app state
#[derive(Debug)]
pub(crate) enum Event {
    ///[`Event`] containing a [`crossterm::event::Event`]
    InputEvent(crossterm::event::Event),
    ///[`Event`] containing a [`Message`]
    MessageEvent(Message),
    ErrorEvent(String),
    ///[`Event`] containing a [`TcpStream`]
    NewStream(TcpStream),
    ///[`Event`] containing an [`Arc<Connection>`]
    ConnectionEvent(Arc<Connection>),
    ///[`Event`] containing the address of a peer that disconnected as a [`String`]
    DisconnectionEvent(String),
    ///[`Event`] containing a listen address as a [`String`],
    ///used for updating the local username when none is set
    ListenEvent(String),
    ///Generic [`Event`] for forcing the app to render
    Update,
}

///Struct to store the app state
#[derive(Debug)]
pub(crate) struct App {
    pub(crate) running: Arc<AtomicBool>,
    pub(crate) connections: Connections,
    pub(crate) messages: Vec<Message>,
    pub(crate) log_file: Option<fs::File>,
    pub(crate) listen_addr: String,
    pub(crate) nick: Nick,
    pub(crate) color: Color,
    pub(crate) input_buf: (String, usize),
    pub(crate) tx: Sender<Event>,
    pub(crate) rx: Receiver<Event>,
    pub(crate) show_peers: bool,
    pub(crate) config: Config,
}

impl App {
    ///Creates a new [`App`] instance with the given [`Config`]
    pub(crate) fn new(config: Config) -> Result<Self> {
        let (tx, rx) = mpsc::channel::<Event>();
        let listen_addr = format!("{}:{}", config.listen_ips[0], config.listen_ports[0]);
        let log_file = if config.log_messages {
            if !fs::exists(&config.log_path)? {
                fs::File::create_new(&config.log_path)?;
            }
            Some(
                fs::OpenOptions::new()
                    .write(true)
                    .append(true)
                    .open(&config.log_path)?,
            )
        } else {
            None
        };
        Ok(App {
            running: Arc::new(AtomicBool::new(true)),
            connections: vec![],
            messages: vec![],
            log_file,
            listen_addr,
            nick: RwLock::new(config.nick.clone()),
            color: random_color(),
            input_buf: (String::new(), 0),
            tx,
            rx,
            show_peers: true,
            config,
        })
    }

    ///Runs [`App`] in `terminal`
    pub(crate) fn run(&mut self, terminal: &mut DefaultTerminal) -> Result<()> {
        let t = self.tx.clone();
        let r = self.running.clone();
        spawn(move || -> Result<()> { input_listener(t, r) });

        if self.config.listen_ips[0] == "all" {
            self.config.listen_ips = local_ipv4_addrs();
        }

        //spawn a listener for all listen_ports on all listen_ips
        for ip in &self.config.listen_ips {
            for port in &self.config.listen_ports {
                let addr = format!("{}:{}", ip, port);
                let t = self.tx.clone();
                //let r = self.running.clone();
                spawn(move || -> Result<()> {
                    connection_listener(t, /*r,*/ addr)
                });
            }
        }

        for addr in &self.config.startup_connections.clone() {
            self.connect(addr)?;
        }

        while self.running.load(Ordering::Relaxed) {
            terminal.draw(|mut frame| self.render(&mut frame))?;
            self.update()?;
        }

        Ok(())
    }

    ///Updates the [`App`] state
    fn update(&mut self) -> Result<()> {
        match self.rx.recv()? {
            InputEvent(event) => {
                self.handle_input(event)?;
            }
            MessageEvent(message) => {
                self.display_msg(message)?;
            }
            ErrorEvent(error) => {
                self.display_error(&error)?;
            }
            NewStream(stream) => {
                let t = self.tx.clone();
                let r = self.running.clone();
                spawn(move || connection_handler(t, r, stream));
            }
            ConnectionEvent(connection) => {
                self.handle_new_connection(connection)?;
            }
            DisconnectionEvent(peer_addr) => {
                self.disconnect(&peer_addr)?;
            }
            ListenEvent(listen_addr) => {
                self.listen_addr = listen_addr;
            }
            Update => (),
        }

        Ok(())
    }

    fn handle_new_connection(&mut self, connection: Arc<Connection>) -> Result<()> {
        self.display_msg(vec![
            ("<".to_string(), Style::new()),
            (
                connection.peer_addr.clone(),
                Style::new().fg(connection.peer_color),
            ),
            ("> ".to_string(), Style::new()),
            ("joined".to_string(), Style::new().dark_gray()),
        ])?;
        if let Some(n) = self.nick.read().unwrap().clone() {
            let c = connection.clone();
            spawn(move || -> Result<()> {
                send_msg(c, Arc::new(format!("/n {n}")), MessageType::Command)
            });
        }
        Ok(self.connections.push(connection))
    }

    ///Handles [crossterm] events, currently only key presses
    fn handle_input(&mut self, event: crossterm::event::Event) -> Result<()> {
        match event {
            crossterm::event::Event::Key(key) => match key.code {
                KeyCode::Esc => {
                    self.running.store(false, Ordering::Relaxed); //exits app
                }
                KeyCode::Tab => {
                    self.show_peers = !self.show_peers;
                }
                KeyCode::Up => {} //todo scrolling
                KeyCode::Down => {}
                KeyCode::Left => {
                    if self.input_buf.1 <= self.input_buf.0.len() {
                        self.input_buf.1 += 1;
                    }
                }
                KeyCode::Right => {
                    if self.input_buf.1 > 0 {
                        self.input_buf.1 -= 1;
                    }
                }
                KeyCode::Char(c) => {
                    //exits the app if ctrl+c is pressed,
                    //handles a hotkey if alt is pressed,
                    //or adds a character to the input buffer
                    if key.modifiers.contains(KeyModifiers::CONTROL) && c == 'c' {
                        self.running.store(false, Ordering::Relaxed);
                    }
                    /*else if key.modifiers.contains(KeyModifiers::ALT) {
                        self.handle_hotkey(c)?;
                    } */
                    else {
                        self.input_buf
                            .0
                            .insert(self.input_buf.0.len() - self.input_buf.1, c);
                    }
                }
                KeyCode::Backspace => {
                    if self.input_buf.0.len() > self.input_buf.1 {
                        let idx = self.input_buf.0.len() - self.input_buf.1 - 1;
                        self.input_buf.0.remove(idx);
                    }
                    if self.input_buf.1 > self.input_buf.0.len() {
                        self.input_buf.1 -= 1;
                    }
                }
                KeyCode::Delete => {
                    if self.input_buf.1 > 0 {
                        let idx = self.input_buf.0.len() - self.input_buf.1;
                        self.input_buf.0.remove(idx);
                        self.input_buf.1 -= 1;
                    }
                }
                KeyCode::Enter => {
                    //parses the input buffer and then clears it
                    self.handle_input_buffer()?;
                    self.input_buf.0.clear();
                    self.input_buf.1 = 0;
                }
                _ => (),
            },
            _ => (),
        }

        Ok(())
    }

    ///Either sends a message or handles a command based on whether the input buffer starts with `/`
    fn handle_input_buffer(&mut self) -> Result<()> {
        match self.input_buf {
            _ if self.input_buf.0.starts_with('/') => self.handle_cmd(),
            _ => {
                self.display_input_msg(MessageType::Text)?;
                self.broadcast_input_msg(MessageType::Text)
            }
        }
    }

    /*///Handles key presses with alt
    fn handle_hotkey(&mut self, key: char) -> Result<()> {
        match key {
            'p' => {
                self.show_peers = !self.show_peers;
            }
            _ => (),
        }
        Ok(())
    }*/

    fn handle_cmd(&mut self) -> Result<()> {
        self.display_input_msg(MessageType::Command)?;
        let binding = self.input_buf.0.clone();
        let mut parts = binding.splitn(2, ' ');
        if let Some(cmd) = parts.next() {
            match cmd {
                //commands that don't have args go here
                "/disconnect_all" | "/da" => {
                    let mut addrs = vec![];
                    for c in &self.connections {
                        addrs.push(c.peer_addr.clone());
                    }
                    for addr in &addrs {
                        self.disconnect(addr)?;
                    }
                    return Ok(());
                }
                _ => (),
            }
            if let Some(arg) = parts.next()
                && !arg.is_empty()
            {
                match cmd {
                    "/nick" | "/n" => {
                        let nick = arg.trim().to_string();
                        self.nick.write().unwrap().replace(nick);
                        self.broadcast_input_msg(MessageType::Command)?;
                    }
                    "/connect" | "/c" => {
                        self.connect(arg.trim())?;
                    }
                    "/disconnect" | "/d" => {
                        if let Some(addr) = self.find_peer_addr(arg.trim()) {
                            self.disconnect(&addr)?;
                        }
                    }
                    "/msg" | "/m" => {
                        let mut args = arg.splitn(2, ' ');
                        if let Some(addr) = args.next()
                            && let Some(msg) = args.next()
                        {
                            if let Some(a) = self.find_peer_addr(&addr)
                                && let Some(c) = self.get_connection(&a)
                            {
                                let m = Arc::new(msg.trim().to_string());
                                spawn(move || -> Result<()> {
                                    send_msg(c, m, MessageType::Text)?;
                                    Ok(())
                                });
                            } else {
                                self.display_error("Failed to send message, no such peer")?;
                            }
                        } else {
                            self.display_error("No message specified")?;
                        }
                    }
                    "/message_file" | "/mf" => {
                        let mut args = arg.splitn(2, ' ');
                        if let Some(addr) = args.next()
                            && let Some(file) = args.next()
                            && !file.is_empty()
                        {
                            if let Some(a) = self.find_peer_addr(&addr)
                                && let Some(c) = self.get_connection(&a)
                            {
                                let p = Arc::new(PathBuf::from(file.trim()));
                                spawn(move || -> Result<()> { send_file(c, p) });
                            } else {
                                self.display_error("Failed to send file, no such peer")?;
                            }
                        } else {
                            self.display_error("No file specified")?;
                        }
                    }
                    "/send_file" | "/sf" => {
                        let path = Path::new(arg);
                        if path.try_exists()? {
                            self.broadcast_file(&path)
                        } else {
                            self.display_error("File does not exist")?;
                        }
                    }
                    _ => self.display_error(&format!("Unknown command: {cmd}"))?,
                }
            } else {
                self.display_error("Command needs an argument")?;
            }
        }

        Ok(())
    }

    fn connect(&mut self, addr: &str) -> Result<()> {
        if self.get_connection(addr).is_some() {
            return self.display_error(&format!("Already connected to {addr}"));
        }
        self.display_msg(vec![(
            format!("Connecting to {}...", addr),
            Style::new().dark_gray(),
        )])?;
        let a = addr.to_string();
        let t = self.tx.clone();
        spawn(move || -> Result<()> {
            let sleep_secs = 3u64;
            for n in 0..CONNECTION_RETRIES {
                if n > 0 {
                    t.send(ErrorEvent(format!(
                        "Failed to connect to {a}, retrying in {sleep_secs} seconds..."
                    )))?;
                    thread::sleep(Duration::from_secs(sleep_secs));
                }
                if let Ok(s) = TcpStream::connect(&a) {
                    return Ok(t.send(NewStream(s))?);
                }
            }
            t.send(ErrorEvent(format!("Failed to connect to {a}")))?;

            Ok(())
        });

        Ok(())
    }

    fn disconnect(&mut self, peer_addr: &str) -> Result<()> {
        let mut disconnected = false;
        let mut nick = String::new();
        let mut color = Color::Reset;
        self.connections.retain(|c| {
            if c.peer_addr == peer_addr {
                let _ = c.stream.shutdown(Shutdown::Both);
                color = c.peer_color.clone();
                nick = c
                    .peer_nick
                    .read()
                    .unwrap()
                    .clone()
                    .unwrap_or_else(|| c.peer_addr.clone());
                disconnected = true;
                false
            } else {
                true
            }
        });

        if disconnected {
            self.display_msg(vec![
                ("<".to_string(), Style::new()),
                (nick, Style::new().fg(color)),
                ("> ".to_string(), Style::new()),
                ("disconnected".to_string(), Style::new().dark_gray()),
            ])?;
        } else {
            self.display_error(&format!(
                "Failed to disconnect from {peer_addr}; no such peer"
            ))?;
        }

        Ok(())
    }

    fn broadcast_input_msg(&mut self, msg_type: MessageType) -> Result<()> {
        let msg = Arc::new(self.input_buf.0.clone());
        for c in &self.connections {
            let c = c.clone();
            let m = msg.clone();
            let t = msg_type.clone();
            spawn(move || -> Result<()> {
                send_msg(c, m, t)?;
                Ok(())
            });
        }

        Ok(())
    }

    fn broadcast_file(&self, path: &Path) {
        let path = Arc::new(path.to_path_buf());
        for c in &self.connections {
            let c = c.clone();
            let p = path.clone();
            spawn(move || -> Result<()> { send_file(c, p) });
        }
    }

    ///Adds a message to the list of messages with the current time appended to the front,
    ///also writes the message to the log if there is one
    fn display_msg(&mut self, msg: Message) -> Result<()> {
        let time = Local::now().format("%H:%M:%S").to_string();
        let mut message = vec![
            (time, Style::new().dark_gray()),
            (" | ".to_string(), Style::new()),
        ];
        message.extend_from_slice(&msg);
        self.log_msg(&message)?;
        Ok(self.messages.push(message))
    }

    ///Writes `msg` to `log_file` if there is one
    fn log_msg(&self, msg: &Message) -> Result<()> {
        if let Some(log) = &self.log_file {
            let mut writer = BufWriter::new(log);
            let mut message = String::new();
            for part in msg {
                message.push_str(&part.0);
            }
            message.push('\n');
            writer.write_all(message.as_bytes())?;
            writer.flush()?;
        }

        Ok(())
    }

    fn display_error(&mut self, error: &str) -> Result<()> {
        self.display_msg(vec![(format!("Error: {error}"), Style::new().red())])
    }

    fn display_input_msg(&mut self, msg_type: MessageType) -> Result<()> {
        let msg = vec![
            ("<".to_string(), Style::new()),
            (
                self.nick
                    .read()
                    .unwrap()
                    .clone()
                    .unwrap_or_else(|| self.listen_addr.clone()),
                Style::new().fg(self.color),
            ),
            ("> ".to_string(), Style::new()),
            (
                self.input_buf.0.clone(),
                Style::new().fg(match msg_type {
                    MessageType::Text => Color::Reset,
                    MessageType::Command => Color::Yellow,
                    _ => Color::DarkGray,
                }),
            ),
        ];
        self.display_msg(msg)
    }

    fn find_peer_addr(&self, peer_nick: &str) -> Option<String> {
        if let Some(c) = self
            .connections
            .iter()
            .find(|c| *c.peer_nick.read().unwrap() == Some(peer_nick.to_string()))
        {
            Some(c.peer_addr.clone())
        } else {
            None
        }
    }

    fn get_connection(&self, peer_addr: &str) -> Option<Arc<Connection>> {
        self.connections
            .iter()
            .find(|c| c.peer_addr == peer_addr)
            .cloned()
    }

    fn render(&self, frame: &mut Frame) {
        frame.render_widget(self, frame.area());
    }
}

impl Widget for &App {
    fn render(self, area: Rect, buf: &mut Buffer) {
        let vertical_layout = Layout::vertical([Constraint::Percentage(100), Constraint::Min(3)]);
        let [mut message_area, input_area] = vertical_layout.areas::<2>(area);
        message_area.height += 1;

        if self.show_peers {
            let horizontal_layout =
                Layout::horizontal([Constraint::Percentage(75), Constraint::Percentage(25)]);
            let [mut m, peer_area] = horizontal_layout.areas::<2>(message_area);
            m.width += 1;
            message_area = m;

            let mut peers = vec![];
            for c in &self.connections {
                let style = Style::new().fg(c.peer_color);
                peers.push(Line::from(
                    if let Some(n) = c.peer_nick.read().unwrap().clone() {
                        vec![
                            Span::raw("<"),
                            Span::styled(n, style),
                            Span::raw("> "),
                            Span::raw("("),
                            Span::styled(&c.peer_addr, style),
                            Span::raw(")"),
                        ]
                    } else {
                        vec![
                            Span::raw("<"),
                            Span::styled(&c.peer_addr, style),
                            Span::raw(">"),
                        ]
                    },
                ))
            }
            let peer_paragraph = Paragraph::new(peers)
                .block(
                    Block::bordered()
                        .title("─┤Peers├")
                        .merge_borders(Fuzzy)
                        .padding(Padding::horizontal(1)),
                )
                .wrap(Wrap { trim: false });
            peer_paragraph.render(peer_area, buf);
        }

        let binding = self.messages.clone();
        let mut messages: Vec<Line> = binding
            .iter()
            .map(|m| {
                let mut line = Line::default();
                for part in m {
                    line.push_span(Span::styled(&part.0, part.1));
                }
                line
            })
            .rev()
            .collect();
        let area_height = message_area.height as usize - 2;
        let area_width = message_area.width as usize - 4;
        messages.truncate(area_height);
        let mut wraps = calculate_wraps(&messages, area_width);
        while (messages.len() + wraps) > area_height {
            messages.truncate(messages.len() - 1);
            wraps = calculate_wraps(&messages, area_width);
        }
        messages.reverse();
        let message_paragraph = Paragraph::new(messages.clone())
            .block(
                Block::bordered()
                    .title("─┤Messages├")
                    .merge_borders(Fuzzy)
                    .padding(Padding::horizontal(1)),
            )
            .wrap(Wrap { trim: false });

        let nick = self
            .nick
            .read()
            .unwrap()
            .clone()
            .unwrap_or_else(|| self.listen_addr.clone());
        let input_layout = Layout::horizontal([
            Constraint::Max(nick.len() as u16 + 5),
            Constraint::Fill(1), //Min(self.input_buf.0.len() as u16 + 3) causes a crash when horizontal size is too small
        ]);
        let [mut nick_area, input_area] = input_layout.areas::<2>(input_area);
        nick_area.width += 1;
        let nick = Paragraph::new(Line::from(vec![
            Span::raw("<"),
            Span::styled(nick, Style::new().fg(self.color)),
            Span::raw(">"),
        ]))
        .block(
            Block::bordered()
                .merge_borders(Fuzzy)
                .padding(Padding::horizontal(1)),
        );

        //underline the character the cursor is on
        let mut i = self.input_buf.0.clone();
        i.push(' ');
        let idx = i.len() - self.input_buf.1;
        let (first, second) = i.split_at(idx - if idx > 0 { 1 } else { 0 });
        let (second, third) = second.split_at(1);

        let input = Paragraph::new(Line::from(vec![
            Span::raw(first),
            Span::styled(second, Style::new().underlined().slow_blink()), //todo make blink actually work
            Span::raw(third),
        ]))
        .block(
            Block::bordered()
                .merge_borders(Fuzzy)
                .padding(Padding::horizontal(1)),
        );

        message_paragraph.render(message_area, buf);
        nick.render(nick_area, buf);
        input.render(input_area, buf);
    }
}

///Sends each input as an [`InputEvent`] to the app
fn input_listener(tx: Sender<Event>, running: Arc<AtomicBool>) -> Result<()> {
    while running.load(Ordering::Relaxed) {
        if let Ok(event) = crossterm::event::read() {
            tx.send(InputEvent(event))?;
        }
    }

    Ok(())
}
