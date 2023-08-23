use std::fmt;

use super::types::*;

#[derive(Debug, Clone)]
pub enum Command {
    CreateAccount(Account),
    UpdateAccount(Id<Account>, Vec<AccountModification>),
    AddTransaction(Transaction),
}

#[derive(Debug, Clone)]
pub enum AccountModification {
    Disable,
    UpdateName(String),
    UpdateNotes(String),
}

impl fmt::Display for Command {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Command::CreateAccount(account) => {
                write!(f, r#"Create account {}: "{}""#, account.id, account.name)
            }
            Command::AddTransaction(transaction) => write!(
                f,
                "Add transaction {} of {}: {}",
                transaction.id,
                transaction.amount,
                match &transaction.inner {
                    TransactionInner::Received { src, .. } => format!("received from {src}"),
                    TransactionInner::Paid { dst, .. } => format!("paid to {dst}"),
                    TransactionInner::MovePhys { dst, .. } => format!("moved to {dst}"),
                    TransactionInner::MoveVirt { dst, .. } => format!("moved to {dst}"),
                    TransactionInner::Convert { new_amount, .. } =>
                        format!("converted to {new_amount}"),
                }
            ),
            Command::UpdateAccount(account, actions) => write!(
                f,
                "Update account {}:\n{}",
                account,
                actions
                    .iter()
                    .map(|x| match x {
                        AccountModification::Disable => "  - disable account\n".to_owned(),
                        AccountModification::UpdateName(name) =>
                            format!("  - set name to \"{}\"\n", name),
                        AccountModification::UpdateNotes(notes) =>
                            format!("  - set notes to \"{}\"\n", notes),
                    })
                    .collect::<String>()
            ),
        }
    }
}
