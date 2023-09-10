mod command;
mod repl;
mod repository;
mod types;

use std::{
    env,
    net::SocketAddr, path::PathBuf,
};

use clap::{Parser, Subcommand};
use eyre::{eyre, Result};
use repository::Repository;
use tracing_subscriber::prelude::*;

#[derive(Parser)]
struct Args {
    #[command(subcommand)]
    subcommand: Option<Command>,
}

#[derive(Subcommand)]
enum Command {
    Init { path: PathBuf },
    Serve {
        #[command(subcommand)]
        mode: ServeMode,
    },
    Run {
        args: Vec<String>,
    },
}

#[derive(Subcommand, Debug)]
pub enum ServeMode {
    /// Serve over stdin/stdout
    Stdio,
    /// Bind to a listening socket ourselves
    Bind { addr: SocketAddr },
    /// Get socket listener from systemd LISTEN_FDS
    Systemd,
}

fn main() -> Result<()> {
    color_eyre::install()?;
    tracing::subscriber::set_global_default(
        tracing_subscriber::registry()
            .with(tracing_subscriber::fmt::layer())
            .with(tracing_subscriber::EnvFilter::from_default_env())
            .with(tracing_error::ErrorLayer::default()),
    )?;

    let Args { subcommand } = Args::parse();
    let repo = env::var_os("MONFARI_REPO").ok_or(eyre!("MONFARI_REPO must be set"))?;
    match subcommand {
        Some(Command::Init { path }) => {
            Repository::init(path)?;
        }
        None => {
            repl::repl(Repository::open(&repo)?)?;
        }
        Some(Command::Run { mut args }) => {
            for arg in &mut args {
                if arg.contains(' ') {
                    *arg = format!("\"{}\"", arg);
                }
            }
            repl::command(Repository::open(&repo)?, args.join(" "))?;
        }
        Some(Command::Serve { mode }) => {
            repository::serve(mode, repo)?;
        }
    }

    Ok(())
}

