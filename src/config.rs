use clap::Parser;
use serde::Deserialize;
use std::env::home_dir;
use std::fs;
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, Deserialize)]
#[serde(default)]
pub(crate) struct Config {
    pub(crate) debug: bool,
    pub(crate) listen_ips: Vec<String>,
    pub(crate) listen_ports: Vec<u16>,
    pub(crate) log_messages: bool,
    pub(crate) log_path: PathBuf,
    pub(crate) nick: Option<String>,
    pub(crate) startup_connections: Vec<String>,
}

impl Config {
    pub(crate) fn parse() -> Self {
        let mut config = Self::default();
        let args = Args::parse();
        let mut file_config = None;
        if !args.no_config {
            if let Some(path) = args.config_path {
                file_config = read_config_file(&path);
            } else {
                let mut config_paths = vec![PathBuf::from("tcp_messenger.toml")];
                if let Some(dir) = home_dir() {
                    #[cfg(target_family = "unix")]
                    config_paths.push(PathBuf::from(dir).join(".config/tcp_messenger/config.toml"));
                    #[cfg(target_family = "windows")]
                    config_paths.push(
                        PathBuf::from(dir).join("AppData\\Roaming\\tcp_messenger\\config.toml")
                    );
                }
                for path in config_paths {
                    if let Some(a) = read_config_file(&path) {
                        file_config = Some(a);
                        break;
                    }
                }
            }
        }
        if let Some(cfg) = file_config {
            config = cfg;
        }

        //would be nice to have a function to do this instead
        if args.debug {
            config.debug = args.debug;
        }
        if let Some(a) = args.listen_ips {
            config.listen_ips = a;
        }
        if let Some(a) = args.listen_ports {
            config.listen_ports = a;
        }
        if args.log_messages {
            config.log_messages = args.log_messages;
        }
        if let Some(a) = args.log_path {
            config.log_path = a;
        }
        if let Some(a) = args.nick {
            config.nick = Some(a);
        }
        if let Some(a) = args.startup_connections {
            config.startup_connections = a;
        }

        config
    }
}

impl Default for Config {
    fn default() -> Self {
        Self {
            debug: false,
            listen_ips: vec!["all".to_string()],
            listen_ports: vec![0],
            log_messages: false,
            log_path: PathBuf::from("messenger.log"),
            nick: None,
            startup_connections: vec![]
        }
    }
}

///Struct for parsing command line arguments with [`clap`]
#[derive(Parser, Debug, Clone)]
struct Args {
    #[arg(short, long)]
    config_path: Option<PathBuf>,
    #[arg(short, long, action)]
    debug: bool,
    #[arg(
        short = 'i', long,
        num_args = 1..,
        value_delimiter = ',',
    )]
    listen_ips: Option<Vec<String>>,
    #[arg(
        short = 'p', long,
        num_args = 1..,
        value_delimiter = ',',
    )]
    listen_ports: Option<Vec<u16>>,
    #[arg(short, long, action)]
    log_messages: bool,
    #[arg(long)]
    log_path: Option<PathBuf>,
    #[arg(short, long, num_args = 1.., value_delimiter = ',')]
    startup_connections: Option<Vec<String>>,
    #[arg(short, long)]
    nick: Option<String>,
    #[arg(long, action)]
    no_config: bool
}

fn read_config_file(path: &Path) -> Option<Config> {
    if let Ok(e) = fs::exists(path) && e && let Ok(config) = fs::read_to_string(path) {
        match toml::from_str::<Config>(&config) {
            Ok(config) => Some(config),
            Err(e) => {
                eprintln!("Error parsing config file: {e}");
                None
            }
        }
    } else {
        None
    }
}
