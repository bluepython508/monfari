#![allow(unused)]
use crate::{
    command::{AccountModification, Command},
    types::{Account, AccountType, Amount, Id, Transaction, TransactionInner},
};
use eyre::Result;
use sqlx::{query, query_as, SqlitePool};
use tracing::instrument;

#[derive(Debug)]
pub(super) struct SqlRepository {
    db: SqlitePool,
}

macro_rules! impl_type {
    ($(<$($typaram:ident),*> $ty:ty);+ $(;)?) => {
        $(
            impl<'a, DB: sqlx::Database, $($typaram),*> sqlx::Decode<'a, DB> for $ty where &'a str: sqlx::Decode<'a, DB> {
                fn decode(value: <DB as sqlx::database::HasValueRef<'a>>::ValueRef) -> Result<Self, sqlx::error::BoxDynError> {
                    Ok(<&'a str as sqlx::Decode<'a, DB>>::decode(value)?.parse()?)
                }
            }
            impl<'a, DB: sqlx::Database, $($typaram),*> sqlx::Encode<'a, DB> for $ty where String: sqlx::Encode<'a, DB> {
                fn encode_by_ref(&self, buf: &mut <DB as sqlx::database::HasArguments<'a>>::ArgumentBuffer) -> sqlx::encode::IsNull {
                    self.to_string().encode(buf)
                }
            }
            impl<DB: sqlx::Database, $($typaram),*> sqlx::Type<DB> for $ty where String: sqlx::Type<DB> {
                fn type_info() -> DB::TypeInfo {
                    String::type_info()
                }
            }
        )+
    }
}

impl_type! {
    <T> Id<T>;
    <> Amount;
}

#[derive(Debug, Eq, PartialEq, sqlx::Type, Copy, Clone)]
enum TransactionType {
    Received,
    Paid,
    MovePhys,
    MoveVirt,
    Convert,
}

#[derive(Debug)]
struct TransactionDb {
    id: Id<Transaction>,
    amount: Amount,
    r#type: TransactionType,
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
            r#type,
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
            inner: match r#type {
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

#[derive(Debug)]
struct AccountDb {
    id: Id<Account>,
    r#type: AccountType,
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
            r#type,
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
            typ: r#type,
            current,
            enabled,
        })
    }
}

impl SqlRepository {
    #[instrument]
    pub async fn open(f: &str) -> Result<Self> {
        let db = SqlitePool::connect(&format!("sqlite:{f}?mode=rwc")).await?;
        sqlx::migrate!().run(&db).await?;

        Ok(Self {
           db,
        })
    }
}

impl SqlRepository {
    #[instrument]
    pub async fn transactions(&self, id: Id<Account>) -> Result<Vec<Transaction>> {
        query_as!(
            TransactionDb,
            r#"
            SELECT
                id as "id: _", 
                amount as "amount: _",
                type as "type: _",
                new_amount as "new_amount?: _",
                external_party,
                acc_1 as "acc_1: _",
                acc_2 as "acc_2: _",
                notes
            FROM transactions
            WHERE acc_1 = ?1 OR acc_2 = ?1
        "#,
            id
        )
        .fetch_all(&self.db)
        .await?
        .into_iter()
        .map(TransactionDb::to_transaction)
        .collect()
    }

    #[instrument]
    pub async fn account(&self, id: Id<Account>) -> Result<Account> {
        let transactions = self.transactions(id).await?;
        query_as!(
            AccountDb,
            r#"
                SELECT
                    id as "id: _",
                    type as "type: _",
                    name,
                    notes,
                    enabled as "enabled: _"
                FROM accounts
                WHERE id = ?
            "#,
            id
        )
        .fetch_optional(&self.db)
        .await?
        .ok_or_else(|| eyre::eyre!("No such account"))?
        .to_account(&transactions[..])
    }

    #[instrument]
    pub async fn accounts(&self) -> Result<Vec<Account>> {
        let accounts = query_as!(
            AccountDb,
            r#"
                SELECT
                    id as "id: _",
                    type as "type: _",
                    name,
                    notes,
                    enabled as "enabled: _"
                FROM accounts
            "#,
        )
        .fetch_all(&self.db)
        .await?
        .into_iter()
        .map(|acc| async move {
            let transactions = self.transactions(acc.id).await?;
            acc.to_account(&transactions)
        })
        .collect::<Vec<_>>();
        futures::future::join_all(accounts)
            .await
            .into_iter()
            .collect()
    }

    #[instrument]
    pub async fn run_command(&self, cmd: Command) -> Result<()> {
        let mut transaction = self.db.begin().await?;

        {
            let id = Id::<Command>::generate();
            let cmd = serde_json::to_string(&cmd)?;
            query!("INSERT INTO commands VALUES (?, ?)", id, cmd)
                .execute(&mut *transaction)
                .await?;
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
                query!(
                    "INSERT INTO accounts(id, name, notes, type, enabled) VALUES (?, ?, ?, ?, ?)",
                    id,
                    name,
                    notes,
                    typ,
                    enabled
                )
                .execute(&mut *transaction)
                .await?;
            }
            Command::UpdateAccount(acc, changes) => {
                for change in changes {
                    match change {
                        AccountModification::Disable => {
                            query!("UPDATE accounts SET enabled = false WHERE id = ?", acc)
                                .execute(&mut *transaction)
                                .await?;
                        }
                        AccountModification::UpdateName(name) => {
                            query!("UPDATE accounts SET name = ? WHERE id = ?", name, acc)
                                .execute(&mut *transaction)
                                .await?;
                        }
                        AccountModification::UpdateNotes(notes) => {
                            query!("UPDATE accounts SET notes = ? WHERE id = ?", notes, acc)
                                .execute(&mut *transaction)
                                .await?;
                        }
                    }
                }
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
                query!("INSERT INTO transactions(id, notes, amount, type, acc_1, acc_2, external_party, new_amount) VALUES (?, ?, ?, ?, ?, ?, ?, ?)",
                                                 id, notes, amount, typ,  acc_1, acc_2, external_party, new_amount
                ).execute(&mut *transaction).await?;
            }
        }

        transaction.commit().await?;
        Ok(())
    }
}
