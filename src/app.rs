use crate::app::AppEvent::*;
use crate::config::Config;
use crate::connections::*;
use chrono::Local;
use color_eyre::Result;
use ratatui::buffer::Buffer;
use ratatui::crossterm::event;
use ratatui::crossterm::event::{
    DisableBracketedPaste, EnableBracketedPaste, Event, KeyCode, KeyModifiers,
};
use ratatui::layout::{Constraint, Layout, Margin, Rect};
use ratatui::prelude::{Line, Widget};
use ratatui::style::{Color, Style};
use ratatui::symbols::merge::MergeStrategy::Fuzzy;
use ratatui::text::Span;
use ratatui::widgets::{
    Block, Padding, Paragraph, Scrollbar, ScrollbarOrientation, ScrollbarState, StatefulWidget,
    Wrap,
};
use ratatui::{DefaultTerminal, Frame};
use size::Size;
use std::cell::Cell;
use std::fmt::Debug;
use std::fs;
use std::io::{stdout, BufWriter, Write};
use std::net::{Shutdown, TcpStream};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{Receiver, Sender};
use std::sync::{mpsc, Arc};
use std::thread::{sleep, spawn, JoinHandle};
use std::time::Duration;
use ratatui::prelude::Color::*;

pub(crate) const COMMAND: Style = Style::new().yellow();
pub(crate) const ERROR: Style = Style::new().red();
pub(crate) const INFO: Style = Style::new().dark_gray();

type Connections = Vec<Arc<Connection>>;

#[derive(Debug)]
pub(crate) struct Download {
    pub(crate) connection: Arc<Connection>,
    pub(crate) id: u64,
    pub(crate) path: String,
    pub(crate) progress: Size,
    pub(crate) size: Size
}

///Events for updating the app state
#[derive(Debug)]
pub(crate) enum AppEvent {
    ///Event containing an [`Event`]
    InputEvent(Event),
    ///Event containing a [`Line<'static>`]
    MessageEvent(Line<'static>),
    ErrorEvent(String),
    ///Event containing a [`TcpStream`]
    NewStream(TcpStream),
    ///Event containing an [`Arc<Connection>`]
    ConnectionEvent(Arc<Connection>),
    ///Event containing the address of a peer that disconnected as a [`String`]
    DisconnectionEvent(String),
    ///Event containing a listen address as a [`String`],
    ///used for updating the local username when none is set
    ListenEvent(String),
    ///Event containing a new [`Download`]
    DownloadEvent(Download),
    ///Event containing a download id and a progress value in bytes as [`u64`]s
    DownloadProgressEvent(u64, u64),
    ///Event containing a download id to remove as [`u64`]
    DownloadCompleteEvent(u64),
    /////Generic event for forcing the app to render
    //Update
}

///Struct to store the app state
#[derive(Debug)]
pub(crate) struct App<'a> {
    color: Color,
    config: Config,
    connections: Connections,
    downloads: Vec<Download>,
    handles: Vec<JoinHandle<Result<()>>>,
    ///`(input, selection index)`
    input_buf: (String, usize),
    log_file: Option<fs::File>,
    listen_addr: String,
    messages: Vec<Line<'a>>,
    nick: Option<String>,
    running: Arc<AtomicBool>,
    rx: Receiver<AppEvent>,
    scroll_pos: Cell<usize>,
    show_peers: bool,
    terminal_size: (u16, u16),
    tx: Sender<AppEvent>
}

impl App<'static> {
    ///Creates a new [`App`] instance with the given [`Config`]
    pub(crate) fn new(config: Config) -> Result<Self> {
        let (tx, rx) = mpsc::channel::<AppEvent>();
        let listen_addr = format!("{}:{}", config.listen_ips[0], config.listen_ports[0]);
        let log_file = if config.log_messages {
            if !fs::exists(&config.log_path)? {
                fs::File::create_new(&config.log_path)?;
            }
            Some(fs::OpenOptions::new().write(true).append(true).open(&config.log_path)?)
        } else {
            None
        };
        Ok(App {
            color: random_color(),
            connections: vec![],
            downloads: vec![],
            handles: vec![],
            input_buf: (String::new(), 0),
            listen_addr,
            log_file,
            messages: vec![],
            nick: config.nick.clone(),
            config,
            running: Arc::new(AtomicBool::new(true)),
            rx,
            scroll_pos: Cell::new(0),
            show_peers: true,
            terminal_size: ratatui::crossterm::terminal::size()?,
            tx
        })
    }

    ///Runs [`App`] in `terminal`
    pub(crate) fn run(&mut self, terminal: &mut DefaultTerminal) -> Result<()> {
        let mut stdout = stdout();
        ratatui::crossterm::execute!(stdout, EnableBracketedPaste)?;
        let t = self.tx.clone();
        let r = self.running.clone();
        self.handles.push(spawn(move || -> Result<()> { input_listener(t, r) }));

        if self.config.listen_ips[0] == "all" {
            self.config.listen_ips = local_ipv4_addrs();
        }

        for ip in &self.config.listen_ips {
            for port in &self.config.listen_ports {
                let addr = format!("{}:{}", ip, port);
                let t = self.tx.clone();
                self.handles.push(spawn(move || -> Result<()> {
                    connection_listener(t, &addr)
                }));
            }
        }

        for addr in &self.config.startup_connections.clone() {
            self.connect(addr)?;
        }

        while self.running.load(Ordering::Relaxed) {
            terminal.draw(|mut frame| self.render(&mut frame))?;
            self.update()?;
        }

        ratatui::crossterm::execute!(stdout, DisableBracketedPaste)?;
        Ok(())
    }

    ///Updates the [`App`] state
    fn update(&mut self) -> Result<()> {
        let recv = self.rx.recv()?;
        if self.config.debug {
            self.debug(&recv)?;
        }
        match recv {
            InputEvent(event) => {
                self.handle_input(&event)?;
            }
            MessageEvent(message) => {
                self.display_msg(&message)?;
            }
            ErrorEvent(error) => {
                self.display_error(&error)?;
            }
            NewStream(stream) => {
                let t = self.tx.clone();
                let r = self.running.clone();
                self.handles.push(spawn(move || connection_handler(t, r, stream)));
            }
            ConnectionEvent(connection) => {
                self.handle_new_connection(connection)?;
            }
            DisconnectionEvent(peer_addr) => {
                self.disconnect(&peer_addr, false)?;
            }
            ListenEvent(listen_addr) => {
                self.listen_addr = listen_addr;
            }
            DownloadEvent(download) => {
                self.downloads.push(download);
            }
            DownloadProgressEvent(id, progress) => {
                if let Some(idx) = self.downloads.iter().position(|d| d.id == id) {
                    let d = &mut self.downloads[idx];
                    d.progress = Size::from_bytes(progress);
                }
            }
            DownloadCompleteEvent(id) => {
                if let Some(idx) = self.downloads.iter().position(|d| d.id == id) {
                    self.downloads.remove(idx);
                }
            }
            //Update => ()
        }

        Ok(())
    }

    fn handle_new_connection(&mut self, connection: Arc<Connection>) -> Result<()> {
        let mut line = connection.display_peer(false);
        line.push_span(" joined");
        self.display_msg(&line)?;
        if let Some(n) = self.nick.clone() {
            let c = connection.clone();
            self.handles.push(spawn(move || -> Result<()> {
                send_msg(c, Arc::new(format!("/n {n}")), &MessageType::Command)
            }));
        }
        Ok(self.connections.push(connection))
    }

    ///Handles [crossterm] events, currently only key presses
    fn handle_input(&mut self, event: &Event) -> Result<()> {
        match event {
            Event::Key(key) => match key.code {
                KeyCode::Esc => {
                    self.running.store(false, Ordering::Relaxed);
                }
                KeyCode::Tab => {
                    self.show_peers = !self.show_peers;
                }
                KeyCode::PageUp => {
                    self.scroll_pos.set(
                        //-2 for the border, -2 for the input box
                        self.scroll_pos.get() + self.terminal_size.1 as usize - 4
                    );
                }
                KeyCode::PageDown => {
                    let scroll_pos = self.scroll_pos.get();
                    //-2 for the border, -2 for the input box
                    let page_size = self.terminal_size.1 as usize - 4;
                    if scroll_pos >= page_size {
                        self.scroll_pos.set(scroll_pos - page_size);
                    } else {
                        self.scroll_pos.set(0);
                    }
                }
                KeyCode::Up => {
                    self.scroll_pos.set(self.scroll_pos.get() + 1);
                }
                KeyCode::Down => {
                    let scroll_pos = self.scroll_pos.get();
                    if scroll_pos > 0 {
                        self.scroll_pos.set(scroll_pos - 1);
                    }
                }
                KeyCode::Left => {
                    if self.input_buf.1 < self.input_buf.0.len() {
                        self.input_buf.1 += 1;
                    }
                    if key.modifiers.contains(KeyModifiers::ALT) {
                        let len = self.input_buf.0.len();
                        let mut idx = len - self.input_buf.1;
                        let split = self.input_buf.0.split_at(idx);
                        if let Some(i) = split.0.rfind(|c| !char::is_whitespace(c))
                            && let s = split.0.split_at(i + 1)
                            && let Some(i) = s.0.rfind(char::is_whitespace) {
                            idx = i + 1;
                        } else {
                            idx = 0;
                        }
                        self.input_buf.1 = len - idx;
                    }
                }
                KeyCode::Right => {
                    if self.input_buf.1 > 0 {
                        self.input_buf.1 -= 1;
                    }
                    if key.modifiers.contains(KeyModifiers::ALT) {
                        let len = self.input_buf.0.len();
                        let mut idx = len - self.input_buf.1;
                        let split = self.input_buf.0.split_at(idx);
                        if let Some(i) = split.1.find(|c| !char::is_whitespace(c))
                            && let s = split.1.split_at(i)
                            && let Some(i) = s.1.find(char::is_whitespace) {
                            idx = s.1.len() - i;
                        } else {
                            idx = 0;
                        }
                        self.input_buf.1 = idx;
                    }
                }
                KeyCode::Char(c) => {
                    if key.modifiers.contains(KeyModifiers::CONTROL) && c == 'c' {
                        self.running.store(false, Ordering::Relaxed);
                    } else {
                        self.input_buf.0.insert(self.input_buf.0.len() - self.input_buf.1, c);
                    }
                }
                KeyCode::Backspace => {
                    if self.input_buf.0.len() > self.input_buf.1 {
                        let idx = self.input_buf.0.len() - self.input_buf.1 - 1;
                        self.input_buf.0.remove(idx);
                    }
                    if key.modifiers.contains(KeyModifiers::ALT) {
                        let len = self.input_buf.0.len();
                        let idx = len - self.input_buf.1;
                        let mut split = self.input_buf.0.split_at(idx);
                        split.0 = split.0.trim_end();
                        if let Some(space_idx) = split.0.rfind(char::is_whitespace) {
                            split.0 = split.0.split_at(space_idx + 1).0;
                        } else {
                            split.0 = "";
                        }
                        self.input_buf.0 = split.0.to_string() + split.1;
                    }
                }
                KeyCode::Delete => {
                    if self.input_buf.1 > 0 {
                        let idx = self.input_buf.0.len() - self.input_buf.1;
                        self.input_buf.0.remove(idx);
                        self.input_buf.1 -= 1;
                    }
                    if key.modifiers.contains(KeyModifiers::ALT) {
                        let len = self.input_buf.0.len();
                        let idx = len - self.input_buf.1;
                        let mut split = self.input_buf.0.split_at(idx);
                        split.1 = split.1.trim_start();
                        if let Some(space_idx) = split.1.find(char::is_whitespace) {
                            split.1 = split.1.split_at(space_idx).1;
                        } else {
                            split.1 = "";
                        }
                        self.input_buf.0 = split.0.to_string() + split.1;
                        self.input_buf.1 -= len - self.input_buf.0.len();
                    }
                }
                KeyCode::Enter => {
                    self.handle_input_buffer()?;
                    self.input_buf.0.clear();
                    self.input_buf.1 = 0;
                }
                _ => ()
            }
            Event::Paste(paste) => {
                self.input_buf.0.insert_str(self.input_buf.0.len() - self.input_buf.1, paste);
            }
            Event::Resize(width, height) => {
                let scroll_pos = self.scroll_pos.get();
                //keep scroll position when resizing
                if scroll_pos > 0 {
                    let height_diff = height.abs_diff(self.terminal_size.1);
                    if height < &self.terminal_size.1 {
                        self.scroll_pos.set(scroll_pos + height_diff as usize);
                    } else if height > &self.terminal_size.1 {
                        if scroll_pos >= height_diff as usize {
                            self.scroll_pos.set(scroll_pos - height_diff as usize);
                        } else {
                            self.scroll_pos.set(0);
                        }
                    }
                }
                self.terminal_size = (*width, *height);
            }
            _ => ()
        }

        Ok(())
    }

    fn handle_input_buffer(&mut self) -> Result<()> {
        match self.input_buf {
            _ if self.input_buf.0.starts_with('/') => self.handle_cmd(),
            _ => {
                self.broadcast_input_msg(&MessageType::Text);
                self.display_input_msg(&MessageType::Text)
            }
        }
    }

    fn handle_cmd(&mut self) -> Result<()> {
        const COMMANDS: [&str; 8] = [
            "/c,  /connect <ADDRESS>",
            "/d,  /disconnect <NICK|ADDRESS>",
            "/da, /disconnect_all",
            "/h,  /help",
            "/m,  /msg <NICK|ADDRESS> <MESSAGE>",
            "/mf, /msg_file <NICK|ADDRESS> <FILEPATH>",
            "/n,  /nick <NICK>",
            "/sf, /send_file <PATH>"
        ];

        self.display_input_msg(&MessageType::Command)?;
        let binding = self.input_buf.0.clone();
        let mut parts = binding.splitn(2, ' ');
        if let Some(cmd) = parts.next() {
            let arg = if let Some(a) = parts.next() && !a.is_empty() {
                Some(a.trim())
            } else {
                None
            };
            match cmd {
                "/connect" | "/c" => {
                    if let Some(a) = arg {
                        self.connect(a.trim())?;
                    } else {
                        self.display_error("No address specified")?;
                    }
                }
                "/disconnect" | "/d" => {
                    if let Some(a) = arg {
                        if let Some(addr) = self.find_peer_addr(a.trim()) {
                            self.disconnect(&addr, true)?;
                        } else {
                            self.display_error("Failed to disconnect, no such peer")?;
                        }
                    } else {
                        self.display_error("No peer specified")?;
                    }
                }
                "/disconnect_all" | "/da" => {
                    let mut addrs = vec![];
                    for c in &self.connections {
                        addrs.push(c.peer_addr.clone());
                    }
                    for addr in &addrs {
                        self.disconnect(addr, true)?;
                    }
                }
                "/help" | "/h" => {
                    for cmd in COMMANDS {
                        self.display_msg(&Line::from(Span::styled(cmd, INFO)))?;
                    }
                }
                "/msg" | "/m" => {
                    if let Some(a) = arg {
                        let mut args = a.splitn(2, ' ');
                        if let Some(addr) = args.next() && let Some(msg) = args.next() {
                            if let Some(a) = self.find_peer_addr(&addr)
                                && let Some(c) = self.get_connection(&a) {
                                let m = Arc::new(msg.trim().to_string());
                                self.handles.push(spawn(move || -> Result<()> {
                                    send_msg(c, m, &MessageType::Text)
                                }));
                            } else {
                                self.display_error("Failed to send message, no such peer")?;
                            }
                        } else {
                            self.display_error("No message specified")?;
                        }
                    } else {
                        self.display_error("No peer specified")?;
                    }
                }
                "/msg_file" | "/mf" => {
                    if let Some(a) = arg {
                        let mut args = a.splitn(2, ' ');
                        if let Some(addr) = args.next()
                            && let Some(file) = args.next()
                            && !file.is_empty() {
                            if let Some(a) = self.find_peer_addr(&addr)
                                && let Some(c) = self.get_connection(&a) {
                                let p = Arc::new(PathBuf::from(file.trim()));
                                self.handles.push(spawn(move || -> Result<()> { send_file(c, p) }));
                            } else {
                                self.display_error("Failed to send file, no such peer")?;
                            }
                        } else {
                            self.display_error("No path specified")?;
                        }
                    } else {
                        self.display_error("No peer specified")?;
                    }
                }
                "/nick" | "/n" => {
                    if let Some(a) = arg {
                        let nick = a.trim().to_string();
                        self.nick.replace(nick);
                        self.broadcast_input_msg(&MessageType::Command);
                    } else {
                        self.display_error("No nick specified")?;
                    }
                }
                "/send_file" | "/sf" => {
                    if let Some(a) = arg {
                        let path = Path::new(a);
                        if path.try_exists()? {
                            self.broadcast_file(&path);
                        } else {
                            self.display_error("No such file")?;
                        }
                    } else {
                        self.display_error("No file specified")?;
                    }
                }
                _ => self.display_error(&format!("Unknown command: {cmd}"))?
            }
        }

        Ok(())
    }

    fn connect(&mut self, addr: &str) -> Result<()> {
        if self.get_connection(addr).is_some() {
            return self.display_error(&format!("Already connected to {addr}"));
        }
        self.display_msg(&Line::from(Span::styled(format!("Connecting to {}...", addr), INFO)))?;
        let a = addr.to_string();
        let t = self.tx.clone();
        self.handles.push(spawn(move || -> Result<()> {
            let sleep_secs = 5u64;
            for n in 0..CONNECTION_RETRIES {
                if n > 0 {
                    t.send(ErrorEvent(format!(
                        "Failed to connect to {a}, retrying in {sleep_secs} seconds..."
                    )))?;
                    sleep(Duration::from_secs(sleep_secs));
                }
                if let Ok(s) = TcpStream::connect(&a) {
                    return Ok(t.send(NewStream(s))?);
                }
            }
            t.send(ErrorEvent(format!("Failed to connect to {a}")))?;

            Ok(())
        }));

        Ok(())
    }

    fn disconnect(&mut self, peer_addr: &str, self_initiated: bool) -> Result<()> {
        let mut disconnected = false;
        let mut message = Line::raw("");
        self.connections.retain(|c| {
            if c.peer_addr == peer_addr {
                let _ = c.stream.shutdown(Shutdown::Both);
                message = c.display_peer(false);
                disconnected = true;

                false
            } else {
                true
            }
        });

        if self_initiated {
            if disconnected {
                let mut msg = Line::from(Span::styled("Disconnected ", INFO));
                msg.spans.extend_from_slice(&message.spans);
                self.display_msg(&msg)?;
            } else {
                self.display_error(&format!(
                    "Failed to disconnect from {peer_addr}; no such peer"
                ))?;
            }
        } else {
            if disconnected {
                message.push_span(Span::styled(" disconnected", INFO));
                self.display_msg(&message)?;
            }
        }

        Ok(())
    }

    fn broadcast_input_msg(&mut self, msg_type: &MessageType) {
        let msg = Arc::new(self.input_buf.0.clone());
        for c in &self.connections {
            let c = c.clone();
            let m = msg.clone();
            let t = msg_type.clone();
            self.handles.push(spawn(move || -> Result<()> { send_msg(c, m, &t) }));
        }
    }

    fn broadcast_file(&mut self, path: &Path) {
        let path = Arc::new(path.to_path_buf());
        for c in &self.connections {
            let c = c.clone();
            let p = path.clone();
            self.handles.push(spawn(move || -> Result<()> { send_file(c, p) }));
        }
    }

    ///Adds a message to the list of messages with the current time appended to the front,
    ///also writes the message to the log if there is one
    fn display_msg(&mut self, msg: &Line<'static>) -> Result<()> {
        let time = Local::now().format("%H:%M:%S").to_string();
        let mut message = Line::from(vec![
            Span::styled(time, INFO),
            Span::raw(" | "),
        ]);
        message.spans.extend_from_slice(&msg.spans);
        if self.scroll_pos.get() > 0 {
            self.scroll_pos.set(self.scroll_pos.get() + 1);
        }
        self.log_msg(&message)?;
        self.messages.push(message.clone());

        Ok(())
    }

    ///Writes `msg` to `log_file` if there is one
    fn log_msg(&self, msg: &Line) -> Result<()> {
        if let Some(log) = &self.log_file {
            let mut writer = BufWriter::new(log);
            let message = msg.to_string() + "\n";
            writer.write_all(message.as_bytes())?;
            writer.flush()?;
        }

        Ok(())
    }

    fn display_error(&mut self, error: &str) -> Result<()> {
        self.display_msg(&Line::from(Span::styled(format!("Error: {error}"), ERROR)))
    }

    fn debug(&mut self, d: &impl Debug) -> Result<()> {
        self.display_msg(&Line::from(Span::styled(format!("{d:?}"), INFO)))
    }

    fn display_input_msg(&mut self, msg_type: &MessageType) -> Result<()> {
        self.display_msg(&Line::from(vec![
            Span::raw("<"),
            Span::styled(
                self.nick.clone().unwrap_or_else(|| self.listen_addr.clone()),
                Style::new().fg(self.color)
            ),
            Span::raw("> "),
            Span::styled(
                self.input_buf.0.clone(),
                match msg_type {
                    MessageType::Text => Style::new(),
                    MessageType::Command => COMMAND,
                    _ => INFO,
                }
            ),
        ]))
    }

    fn find_peer_addr(&self, peer_nick: &str) -> Option<String> {
        if let Some(c) = self.connections.iter().find(
            |c| *c.peer_nick.read().unwrap() == Some(peer_nick.to_string())
        ) {
            Some(c.peer_addr.clone())
        } else {
            None
        }
    }

    fn get_connection(&self, peer_addr: &str) -> Option<Arc<Connection>> {
        self.connections.iter().find(|c| c.peer_addr == peer_addr).cloned()
    }

    fn render(&self, frame: &mut Frame) {
        frame.render_widget(self, frame.area());
    }
}

impl Widget for &App<'static> {
    fn render(self, area: Rect, buf: &mut Buffer) {
        let scrolling = self.scroll_pos.get() > 0;
        let vertical_layout = Layout::vertical([Constraint::Percentage(100), Constraint::Min(3)]);
        let [mut message_area, input_area] = vertical_layout.areas::<2>(area);
        message_area.height += 1; //overlap borders

        if self.show_peers {
            let horizontal_layout =
                Layout::horizontal([Constraint::Percentage(75), Constraint::Percentage(25)]);
            let [mut m, peer_area] = horizontal_layout.areas::<2>(message_area);
            m.width += 1; //overlap borders
            message_area = m;

            let mut peers = vec![];
            for c in &self.connections {
                peers.push(c.display_peer(true))
            }
            let peer_paragraph = Paragraph::new(peers).block(
                Block::bordered().title("─┤Peers├").merge_borders(Fuzzy).padding(
                    Padding::horizontal(1)
                )
            ).wrap(Wrap { trim: false });
            peer_paragraph.render(peer_area, buf);
        }

        if !self.downloads.is_empty() {
            let mut downloads: Vec<_> = self.downloads.iter().map(|d| {
                let mut msg = d.connection.display_peer(false);
                msg.push_span(format!(" \"{}\": {}/{}", d.path, d.progress, d.size));
                msg
            }).collect();
            downloads = wrap_lines(downloads, message_area.width as usize - 4);
            let vertical_layout = Layout::vertical([
                Constraint::Min(downloads.len() as u16 + 1),
                Constraint::Percentage(100),
            ]).split(message_area);
            let mut download_area = vertical_layout[0];
            download_area.height += 1;
            message_area = vertical_layout[1];

            let download_paragraph = Paragraph::new(downloads).block(
                Block::bordered().title("─┤Downloads├").merge_borders(Fuzzy).padding(
                    Padding::horizontal(1)
                )
            );
            download_paragraph.render(download_area, buf);
        }

        //the -2 is to account for the border
        let messages_height = message_area.height as usize - 2;
        //4 accounts for the border + padding, 1 extra for the scrollbar if it's visible
        let messages_width = message_area.width as usize - if scrolling { 5 } else { 4 };
        let messages: Vec<Line> = wrap_lines(self.messages.clone(), messages_width);

        let scroll_max = if messages.len() >= messages_height {
            messages.len() - messages_height
        } else {
            0
        };
        if self.scroll_pos.get() > scroll_max {
            self.scroll_pos.set(scroll_max);
        }
        let scroll_pos = self.scroll_pos.get();

        let message_paragraph = Paragraph::new(messages).block(
            Block::bordered().title("─┤Messages├").merge_borders(Fuzzy).padding(Padding {
                left: 1,
                //make space for the scrollbar if it's visible
                right: if scrolling { 2 } else { 1 },
                top: 0,
                bottom: 0
            })
        ).wrap(Wrap { trim: false }).scroll(((scroll_max - scroll_pos) as u16, 0));

        let nick = self.nick.clone().unwrap_or_else(|| self.listen_addr.clone());
        let input_layout = Layout::horizontal([
            Constraint::Max(nick.len() as u16 + 5),
            Constraint::Fill(1),
            //Min(self.input_buf.0.len() as u16 + 3) would be preferable to Fill(1),
            //but causes a crash when the horizontal size is too small
        ]);
        let [mut nick_area, input_area] = input_layout.areas::<2>(input_area);
        nick_area.width += 1;
        let nick = Paragraph::new(Line::from(vec![
            Span::raw("<"),
            Span::styled(nick, Style::new().fg(self.color)),
            Span::raw(">"),
        ])).block(Block::bordered().merge_borders(Fuzzy).padding(Padding::horizontal(1)));

        //underline the character the cursor is on
        let mut i = self.input_buf.0.clone();
        i.push(' ');
        let idx = i.len() - self.input_buf.1;
        let (first, second) = i.split_at(idx - if idx > 0 { 1 } else { 0 });
        let (second, third) = second.split_at(1);

        let input = Paragraph::new(Line::from(vec![
            Span::raw(first),
            //blinking doesn't work on certain terminals
            Span::styled(second, Style::new().underlined().slow_blink()),
            Span::raw(third),
        ])).block(Block::bordered().merge_borders(Fuzzy).padding(Padding::horizontal(1)));

        if scrolling {
            let scrollbar = Scrollbar::new(ScrollbarOrientation::VerticalRight).track_symbol(None)
                .begin_symbol(None)
                .end_symbol(None)
                .thumb_symbol("▐");
            let mut scrollbar_state = ScrollbarState::new(scroll_max).position(
                scroll_max - scroll_pos
            );
            scrollbar.render(
                message_area.inner(Margin::new(1, 1)),
                buf,
                &mut scrollbar_state
            );
        }
        message_paragraph.render(message_area, buf);
        nick.render(nick_area, buf);
        input.render(input_area, buf);
    }
}

///Still kinda awful, but manually wrapping lines gives a much more predictable output
///and makes scrolling work properly
///
///Ratatui's [`Wrap`] *almost* just works out of the box,
///but unfortunately [`Paragraph::scroll()`] relies on there being as many
///actual lines on the terminal as there are [`Line`]`s` in the [`Paragraph`]
///to actually be able to display the whole thing
fn wrap_lines(lines: Vec<Line>, area_width: usize) -> Vec<Line> {
    let mut output = vec![];
    let mut line;
    for l in lines {
        let mut line_width = 0;
        line = Line::default();
        for span in l {
            let mut first = true;
            for part in span.content.split(' ') {
                if !first {
                    line_width += 1;
                    if line_width > area_width {
                        output.push(line.clone());
                        line = Line::default();
                        line_width = 1;
                    }
                    line.push_span(Span::styled(" ", span.style));
                }
                line_width += part.len();
                if line_width > area_width {
                    output.push(line.clone());
                    line = Line::default();
                    line_width = part.len();
                    if part.len() > area_width {
                        let (mut part_1, mut part_2) = part.split_at(area_width);
                        line.push_span(Span::styled(part_1.to_string(), span.style));
                        output.push(line.clone());
                        line = Line::default();
                        while part_2.len() > area_width {
                            (part_1, part_2) = part_2.split_at(area_width);
                            line.push_span(Span::styled(part_1.to_string(), span.style));
                            output.push(line.clone());
                            line = Line::default();
                        }
                        line.push_span(Span::styled(part_2.to_string(), span.style));
                        output.push(line.clone());
                        line = Line::default();
                    } else {
                        line.push_span(Span::styled(part.to_string(), span.style));
                    }
                } else {
                    line.push_span(Span::styled(part.to_string(), span.style));
                }
                first = false;
            }
        }
        if line != Line::default() {
            output.push(line.clone());
        }
    }

    output
}

///Sends each input as an [`InputEvent`] to the app
fn input_listener(tx: Sender<AppEvent>, running: Arc<AtomicBool>) -> Result<()> {
    while running.load(Ordering::Relaxed) {
        if let Ok(event) = event::read() {
            tx.send(InputEvent(event))?;
        }
    }

    Ok(())
}

///Returns a random non-monochrome [`Color`]
pub(crate) fn random_color() -> Color {
    let colors = [
        Blue,
        Cyan,
        Green,
        LightRed,
        LightGreen,
        LightYellow,
        LightBlue,
        LightMagenta,
        LightCyan,
        Magenta,
        Red,
        Yellow,
    ];

    fastrand::choice(colors).unwrap()
}
