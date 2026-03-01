mod app;
mod config;
mod connections;
mod encryption;
mod functions;
mod types;

use crate::app::App;
use crate::config::Config;
use color_eyre::Result;

fn main() -> Result<()> {
    color_eyre::install()?;
    let config = Config::parse();
    let mut terminal = ratatui::init();
    let result = App::new(config)?.run(&mut terminal);
    ratatui::restore();
    result
}
