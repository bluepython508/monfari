#![allow(dead_code)]

mod command;
mod repository;
mod types;

use command::Command;
use types::*;

use tracing_subscriber::prelude::*;

fn main() -> eyre::Result<()> {
    color_eyre::install()?;
    tracing::subscriber::set_global_default(
        tracing_subscriber::registry()
            .with(tracing_subscriber::fmt::layer())
            .with(tracing_subscriber::EnvFilter::from_default_env())
            .with(tracing_error::ErrorLayer::default()),
    )?;

    let mut repo = repository::Repository::init(std::env::args().nth(1).unwrap().into())?;
    let acc_phys_id = Id::generate();
    let acc_virt_id = Id::generate();
    let commands = [
        Command::CreateAccount(Account {
            id: acc_phys_id,
            name: "A Clever Account Name".to_owned(),
            notes: "".to_owned(),
            typ: AccountType::Physical,
            current: Default::default(),
            enabled: true,
        }),
        Command::CreateAccount(Account {
            id: acc_virt_id,
            name: "A default virtual account".to_owned(),
            notes: "".to_owned(),
            typ: AccountType::Virtual,
            current: Default::default(),
            enabled: true,
        }),
        Command::AddTransaction(Transaction {
            id: Id::generate(),
            notes: "".to_owned(),
            amount: "100.00 EUR".parse().unwrap(),
            inner: TransactionInner::Received {
                src: "An Example Source".to_owned(),
                dst: acc_phys_id.erase().unerase(),
                dst_virt: acc_virt_id.erase().unerase(),
            },
        }),
    ];
    for command in commands {
        repo.run_command(command)?;
    }

    Ok(())
}
