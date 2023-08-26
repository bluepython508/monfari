#![allow(dead_code)]

mod command;
mod repl;
mod repository;
mod types;

use std::path::PathBuf;

use clap::{Parser, Subcommand, ValueHint};
use repository::Repository;
use tracing_subscriber::prelude::*;

#[derive(Parser)]
struct Args {
    #[arg(short, long, env = "MONFARI_REPO", value_hint = ValueHint::DirPath)]
    repo: PathBuf,
    #[command(subcommand)]
    subcommand: Option<Command>,
}

#[derive(Subcommand)]
enum Command {
    Clone { url: String },
    Init,
}

fn main() -> eyre::Result<()> {
    color_eyre::install()?;
    tracing::subscriber::set_global_default(
        tracing_subscriber::registry()
            .with(tracing_subscriber::fmt::layer())
            .with(tracing_subscriber::EnvFilter::from_default_env())
            .with(tracing_error::ErrorLayer::default()),
    )?;

    let Args { repo, subcommand } = Args::parse();
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
    }

    Ok(())
}
