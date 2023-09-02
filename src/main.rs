#![allow(dead_code)]

mod command;
mod repl;
mod repository;
mod types;

use std::{
    env,
    ffi::OsString,
    net::{SocketAddr, TcpStream},
    path::Path,
};

use clap::{Parser, Subcommand};
use eyre::{eyre, Result};
use repository::Repository;
use tracing::instrument;
use tracing_subscriber::prelude::*;

#[derive(Parser)]
struct Args {
    #[command(subcommand)]
    subcommand: Option<Command>,
}

#[derive(Subcommand)]
enum Command {
    Init,
    Serve { addr: SocketAddr },
    Run { args: Vec<String> },
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
        Some(Command::Init) => {
            Repository::init(repo.into())?;
        }
        None => {
            repl::repl(open(repo)?)?;
        }
        Some(Command::Run { mut args }) => {
            for arg in &mut args {
                if arg.contains(' ') {
                    *arg = format!("\"{}\"", arg);
                }
            }
            repl::command(open(repo)?, args.join(" "))?;
        }
        Some(Command::Serve { addr }) => {
            repository::serve(addr, repo)?;
        }
    }

    Ok(())
}

#[instrument]
fn open(addr: OsString) -> Result<Repository> {
    if Path::new(&addr).exists() {
        return Repository::open(addr.into());
    }
    Repository::connect(TcpStream::connect(
        addr.to_str()
            .ok_or(eyre!("Expected valid UTF-8 to connect to"))?,
    )?)
}
