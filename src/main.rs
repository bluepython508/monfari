#![allow(dead_code)]

mod command;
mod repl;
mod repository;
mod types;

use std::{path::PathBuf, env};

use clap::{Parser, Subcommand};
use eyre::eyre;
use repository::Repository;
use tracing_subscriber::prelude::*;

#[derive(Parser)]
struct Args {
    #[command(subcommand)]
    subcommand: Option<Command>,
}

#[derive(Subcommand)]
enum Command {
    Clone { url: String },
    Init,
    Run { args: Vec<String> },
}

fn main() -> eyre::Result<()> {
    color_eyre::install()?;
    tracing::subscriber::set_global_default(
        tracing_subscriber::registry()
            .with(tracing_subscriber::fmt::layer())
            .with(tracing_subscriber::EnvFilter::from_default_env())
            .with(tracing_error::ErrorLayer::default()),
    )?;

    let Args { subcommand } = Args::parse();
    let repo: PathBuf = env::var_os("MONFARI_REPO").ok_or(eyre!("MONFARI_REPO must be set"))?.into();
    match subcommand {
        None => {
            repl::repl(Repository::open(repo)?)?;
        }
        Some(Command::Init) => {
            Repository::init(repo)?;
        }
        Some(Command::Clone { url }) => {
            Repository::clone(url, repo)?;
        }
        Some(Command::Run { mut args }) => {
            for arg in &mut args {
                if arg.contains(' ') {
                    arg.push('"'); arg.insert(0, '"')
                }
            }
            repl::command(Repository::open(repo)?, args.join(" "))?;
        }
    }

    Ok(())
}
