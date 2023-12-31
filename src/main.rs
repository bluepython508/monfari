mod command;
mod repl;
mod repository;
mod types;

use std::collections::BTreeMap;
use std::{env, io, net::SocketAddr, path::PathBuf};

use clap::{Parser, Subcommand};
use eyre::{eyre, Result};
use repository::Repository;
use tracing_subscriber::fmt::format::FmtSpan;
use tracing_subscriber::prelude::*;
use tracing_subscriber::{fmt, registry, EnvFilter};

#[derive(Parser)]
struct Args {
    #[command(subcommand)]
    subcommand: Option<Command>,
}

#[derive(Subcommand)]
enum Command {
    Init {
        path: PathBuf,
    },
    Serve {
        #[command(subcommand)]
        mode: ServeMode,
    },
    Run {
        args: Vec<String>,
    },
    Export,
    Import,
}

#[derive(Subcommand, Debug)]
pub enum ServeMode {
    /// Serve over stdin/stdout
    Stdio,
    /// Bind to a listening socket ourselves
    Bind { addr: SocketAddr },
    /// Listen over HTTP
    Http { addr: String },
    /// Get socket listener from systemd LISTEN_FDS
    #[cfg(unix)]
    Systemd,
}

fn main() -> Result<()> {
    color_eyre::install()?;
    tracing::subscriber::set_global_default(
        registry()
            .with(
                fmt::layer()
                    .event_format(fmt::format().with_ansi(true).pretty())
                    .with_span_events(FmtSpan::ACTIVE)
                    .with_writer(io::stderr),
            )
            .with(EnvFilter::from_default_env())
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
        Some(Command::Export) => {
            let repo = Repository::open(&repo)?;
            let accounts = repo.accounts()?;
            let mut transactions = BTreeMap::default();
            for account in &accounts {
                transactions.extend(
                    repo.transactions(account.id)?
                        .into_iter()
                        .map(|x| (x.id, command::Command::AddTransaction(x))),
                );
            }
            println!(
                "{}",
                serde_json::to_string(
                    &accounts
                        .into_iter()
                        .map(|mut acc| {
                            acc.current = Default::default();
                            command::Command::CreateAccount(acc)
                        })
                        .chain(transactions.into_values())
                        .collect::<Vec<_>>()
                )?
            )
        }
        Some(Command::Import) => {
            let mut repo = Repository::open(&repo)?;
            for command in serde_json::from_reader::<_, Vec<command::Command>>(io::stdin())? {
                repo.run_command(command)?;
            }
        }
    }

    Ok(())
}
