use std::{fmt::Display, str::FromStr};

use crate::{
    command::{AccountModification, Command},
    types::{Account, AccountType, Amount, Id, Transaction, TransactionInner},
};
use exemplar::Model;
use eyre::{Result, bail};
use rusqlite::{
    params, params_from_iter,
    types::{FromSql, FromSqlError},
    Connection, ToSql,
};
use rusqlite_migration::{Migrations, M};
use tracing::instrument;

#[derive(Debug)]
pub(super) struct SqlRepository {
    db: Connection,
}

macro_rules! to_from_sql {
    ($($t:ident$(<$($arg:ident),+>)?;)*) => {
        $(
            impl$(<$($arg),+>)? FromSql for $t$(<$($arg),+>)? {
                fn column_result(value: rusqlite::types::ValueRef<'_>) -> rusqlite::types::FromSqlResult<Self> {
                    value.as_str()?.parse::<Self>().map_err(|err| FromSqlError::Other(err.into()))
                }
            }
            impl$(<$($arg),+>)? ToSql for $t$(<$($arg),+>)? {
                fn to_sql(&self) -> rusqlite::Result<rusqlite::types::ToSqlOutput<'_>> {
                    Ok(self.to_string().into())
                }
            }
        )*
    }
}

to_from_sql! {
    Id<T>;
    Amount;
    AccountType;
    TransactionType;
}

#[derive(Copy, Clone, Eq, PartialEq, Hash, Debug)]
enum TransactionType {
    Received,
    Paid,
    MovePhys,
    MoveVirt,
    Convert,
}

impl Display for TransactionType {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            TransactionType::Received => "Received",
            TransactionType::Paid => "Paid",
            TransactionType::MovePhys => "MovePhys",
            TransactionType::MoveVirt => "MoveVirt",
            TransactionType::Convert => "Convert",
        }.fmt(f)
    }
}

impl FromStr for TransactionType {
    type Err = eyre::Report;

    fn from_str(s: &str) -> Result<Self> {
        Ok(match s {
            "Received" => Self::Received,
            "Paid" => Self::Paid,
            "MovePhys" => Self::MovePhys,
            "MoveVirt" => Self::MoveVirt,
            "Convert" => Self::Convert,
            s => bail!("Invalid transaction_type {s}")
        })
    }
}

#[derive(Debug, Model)]
#[table("transactions")]
struct TransactionDb {
    id: Id<Transaction>,
    amount: Amount,
    #[column("type")]
    typ: TransactionType,
    new_amount: Option<Amount>,
    external_party: Option<String>,
    acc_1: Id<Account>,
    acc_2: Id<Account>,
    notes: String,
}

impl TransactionDb {
    #[instrument]
    fn to_transaction(self) -> Result<Transaction> {
        let TransactionDb {
            id,
            amount,
            typ,
            new_amount,
            external_party,
            acc_1,
            acc_2,
            notes,
        } = self;
        Ok(Transaction {
            id,
            notes,
            amount,
            inner: match typ {
                TransactionType::Received => TransactionInner::Received {
                    src: external_party.ok_or_else(|| {
                        eyre::eyre!("`external_party` is required for `received` transactions")
                    })?,
                    dst: acc_1.unerase(),
                    dst_virt: acc_2.unerase(),
                },
                TransactionType::Paid => TransactionInner::Paid {
                    dst: external_party.ok_or_else(|| {
                        eyre::eyre!("`external_party` is required for `paid` transactions")
                    })?,
                    src: acc_1.unerase(),
                    src_virt: acc_2.unerase(),
                },
                TransactionType::MovePhys => TransactionInner::MovePhys {
                    src: acc_1.unerase(),
                    dst: acc_2.unerase(),
                },
                TransactionType::MoveVirt => TransactionInner::MoveVirt {
                    src: acc_1.unerase(),
                    dst: acc_2.unerase(),
                },
                TransactionType::Convert => TransactionInner::Convert {
                    acc: acc_1.unerase(),
                    acc_virt: acc_2.unerase(),
                    new_amount: new_amount.ok_or_else(|| {
                        eyre::eyre!("`new_amount` is required for `convert` transactions")
                    })?,
                },
            },
        })
    }
}

#[derive(Debug, Model)]
#[table("accounts")]
struct AccountDb {
    id: Id<Account>,
    #[column("type")]
    typ: AccountType,
    name: String,
    notes: String,
    enabled: bool,
}

impl AccountDb {
    #[instrument(skip(transactions))]
    fn to_account<'a>(
        self,
        transactions: impl IntoIterator<Item = &'a Transaction>,
    ) -> Result<Account> {
        let AccountDb {
            id,
            typ,
            name,
            notes,
            enabled,
        } = self;
        let current = transactions
            .into_iter()
            .flat_map(|t| {
                t.results()
                    .into_iter()
                    .filter(|(acc, _)| acc == &id)
                    .map(|(_, amount)| amount)
            })
            .sum();
        Ok(Account {
            id,
            name,
            notes,
            typ,
            current,
            enabled,
        })
    }
}

const MIGRATIONS: &[M] = &[M::up(
    r#"
        CREATE TABLE accounts (
        	id TEXT NOT NULL PRIMARY KEY,
        	type TEXT NOT NULL,
        	name TEXT NOT NULL,
        	notes TEXT NOT NULL DEFAULT '',
        	enabled INT NOT NULL DEFAULT TRUE
        ) STRICT;

        CREATE TABLE transactions (
        	id TEXT NOT NULL PRIMARY KEY,
        	amount TEXT NOT NULL,
        	type TEXT NOT NULL, -- Received, Paid, MovePhys, MoveVirt, Convert
        	new_amount TEXT, -- Convert only
        	external_party TEXT, -- src ordst for received and paid respectively
        	acc_1 TEXT NOT NULL REFERENCES accounts (id), -- phys acc for {,_virt} types, src for moves
        	acc_2 TEXT NOT NULL REFERENCES accounts (id), -- virt acc for {,_virt} types, dst for moves
        	notes TEXT NOT NULL DEFAULT ''
        ) STRICT;

        CREATE TABLE commands (
        	id TEXT NOT NULL PRIMARY KEY,
        	command TEXT NOT NULL
        ) STRICT;
    "#,
)];

impl SqlRepository {
    #[instrument]
    pub fn open(f: &str) -> Result<Self> {
        let mut db = Connection::open(f)?;
        db.pragma_update(None, "journal_mode", "WAL")?;

        MIGRATIONS
            .iter()
            .cloned()
            .collect::<Migrations>()
            .to_latest(&mut db)?;

        Ok(Self { db })
    }
}

impl SqlRepository {
    #[instrument]
    pub fn transactions(&self, id: Id<Account>) -> Result<Vec<Transaction>> {
        self.db
            .prepare(
                r#"
            SELECT
                id, 
                amount,
                type,
                new_amount,
                external_party,
                acc_1,
                acc_2,
                notes
            FROM transactions
            WHERE acc_1 = ?1 OR acc_2 = ?1
        "#,
            )?
            .query_and_then(params![id], TransactionDb::from_row)?
            .map(|x| x?.to_transaction())
            .collect()
    }

    #[instrument]
    pub fn account(&self, id: Id<Account>) -> Result<Account> {
        let transactions = self.transactions(id)?;
        self.db
            .query_row(
                r#"
                SELECT
                    id,
                    type,
                    name,
                    notes,
                    enabled
                FROM accounts
                WHERE id = ?
            "#,
                params![id],
                AccountDb::from_row,
            )?
            .to_account(&transactions)
    }

    #[instrument]
    pub fn accounts(&self) -> Result<Vec<Account>> {
        self.db
            .prepare(
                r#"
                SELECT
                    id,
                    type,
                    name,
                    notes,
                    enabled
                FROM accounts
            "#,
            )?
            .query_and_then(params![], AccountDb::from_row)?
            .map(|acc| {
                let acc = acc?;
                let transactions = self.transactions(acc.id)?;
                acc.to_account(&transactions)
            })
            .collect()
    }
    pub fn run_command(&mut self, cmd: Command) -> Result<()> {
        let transaction = self.db.transaction()?;

        {
            let id = Id::<Command>::generate();
            let cmd = serde_json::to_string(&cmd)?;
            transaction.execute("INSERT INTO commands VALUES (?, ?)", params![id, cmd])?;
        };
        match cmd {
            Command::CreateAccount(Account {
                id,
                name,
                notes,
                typ,
                enabled,
                current: _,
            }) => {
                AccountDb {
                    id,
                    name,
                    notes,
                    typ,
                    enabled,
                }
                .insert(&transaction)?;
            }
            Command::UpdateAccount(acc, changes) => {
                let (columns, mut values) = changes
                    .into_iter()
                    .map(|x| match x {
                        AccountModification::Disable => {
                            ("enabled", Box::new(false) as Box<dyn ToSql>)
                        }
                        AccountModification::UpdateName(name) => ("name", Box::new(name) as _),
                        AccountModification::UpdateNotes(notes) => ("notes", Box::new(notes) as _),
                    })
                    .unzip::<_, _, Vec<_>, Vec<_>>();
                values.push(Box::new(acc) as _);
                transaction.execute(
                    &format!(
                        "UPDATE accounts SET {} WHERE id = ?",
                        columns
                            .into_iter()
                            .map(|x| format!("{x} = ?"))
                            .collect::<Vec<String>>()
                            .join(", ")
                    ),
                    params_from_iter(values),
                )?;
            }
            Command::AddTransaction(Transaction {
                id,
                notes,
                amount,
                inner,
            }) => {
                let (typ, acc_1, acc_2, external_party, new_amount) = match inner {
                    TransactionInner::Received { src, dst, dst_virt } => (
                        TransactionType::Received,
                        dst.erase(),
                        dst_virt.erase(),
                        Some(src),
                        None,
                    ),
                    TransactionInner::Paid { src, src_virt, dst } => (
                        TransactionType::Paid,
                        src.erase(),
                        src_virt.erase(),
                        Some(dst),
                        None,
                    ),
                    TransactionInner::MovePhys { src, dst } => (
                        TransactionType::MovePhys,
                        src.erase(),
                        dst.erase(),
                        None,
                        None,
                    ),
                    TransactionInner::MoveVirt { src, dst } => (
                        TransactionType::MoveVirt,
                        src.erase(),
                        dst.erase(),
                        None,
                        None,
                    ),
                    TransactionInner::Convert {
                        acc,
                        acc_virt,
                        new_amount,
                    } => (
                        TransactionType::Convert,
                        acc.erase(),
                        acc_virt.erase(),
                        None,
                        Some(new_amount),
                    ),
                };
                TransactionDb {
                    id,
                    amount,
                    typ,
                    new_amount,
                    external_party,
                    acc_1,
                    acc_2,
                    notes,
                }
                .insert(&transaction)?;
            }
        }

        transaction.commit()?;
        Ok(())
    }
}
