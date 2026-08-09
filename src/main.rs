mod amount;
mod cli;
mod commands;
mod completion;
mod config;
mod display;
mod food;
mod log;

use anyhow::Result;
use clap::{CommandFactory, Parser};
use clap_complete::CompleteEnv;
use cli::Cli;
use config::Config;

fn main() -> Result<()> {
    CompleteEnv::with_factory(Cli::command)
        .completer("intake")
        .complete();

    let Cli {
        command,
        foods_dir,
        log_dir,
    } = Cli::parse();
    display::init_color();
    let config = Config::resolve(foods_dir, log_dir)?;
    commands::run(command, &config)
}
