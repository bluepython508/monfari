use std::{
    fmt::Debug,
    fs,
    path::{Path, PathBuf},
    process,
};

use eyre::{ensure, eyre, Result, WrapErr};
use serde::{Deserialize, Serialize};
use tracing::{debug, instrument};

use crate::{command::*, types::*};

#[instrument]
fn cmd(cmd: &mut process::Command) -> Result<()> {
    let status = cmd.status()?;
    debug!(?status);
    ensure!(
        status.success(),
        "Command {cmd:?} did not exist successfully"
    );
    Ok(())
}

macro_rules! cmd {
    ($cmd:expr $(, $args:expr)* $(,)?) => {
        cmd(
            process::Command::new($cmd)
                $(.arg($args))*
        )
    }
}

macro_rules! git {
    ($(in $dir:expr,)? $($args:expr),* $(,)?) => {
        cmd!("git", $("-C", $dir,)? $($args),*)
    }
}

#[derive(Debug)]
pub struct Repository {
    path: PathBuf,
}

impl Repository {
    #[instrument]
    pub fn init(path: PathBuf) -> Result<Self> {
        const FLAKE_NIX_TMPL: &str = r#"
            {
                description = "Monfari Repo";
                inputs.monfari.url = "github:bluepython508/monfari";
                outputs = { self, monfari }: {
                    inherit (monfari) apps;
                };
            }
        "#;
        const FLAKE_URL: &str = "/home/ben/code/monfari";
        /* Structure:
            - flake.nix
            - accounts
              - {ID}.toml
            - transactions
              - {ID}.toml

            where ID is formatted as proquints
        */
        if path.try_exists()? {
            ensure!(
                path.read_dir()?.next().is_none(),
                "Path must be an empty or non-existent directory"
            );
        } else {
            fs::create_dir_all(&path)?;
        }
        fs::write(path.join("flake.nix"), FLAKE_NIX_TMPL)?;
        for dir in ["transactions", "accounts"] {
            let p = path.join(dir);
            fs::create_dir_all(&p)?;
            fs::File::create(p.join(".gitkeep"))?;
        }

        git!(in &path, "init")?;
        git!(in &path, "add", "transactions", "accounts", "flake.nix")?;
        cmd!(
            "nix",
            "flake",
            "lock",
            "--override-input",
            "monfari",
            FLAKE_URL,
            &path
        )?;
        git!(in &path, "add", "flake.lock")?;

        let mut this = Self { path };
        this.create_account(Account {
            id: Id::generate(),
            name: "Default Virtual Account".to_owned(),
            notes: "A virtual account is required to do much, but many transactions don't really need one, so this is a default to use".to_owned(),
            typ: AccountType::Virtual,
            current: Default::default(),
            enabled: true,
        })?;

        git!(in &this.path, "commit", "-m", "Initial Commit")?;
        Ok(this)
    }

    #[instrument]
    pub fn clone(url: String, path: PathBuf) -> Result<Self> {
        git!("clone", &url, &path)?;
        Self::open(path)
    }

    #[instrument]
    pub fn open(path: PathBuf) -> Result<Self> {
        git!(in &path, "status").wrap_err("git isn't initialized")?;
        ensure!(path.join("accounts").is_dir(), "Not initialized");
        ensure!(path.join("transactions").is_dir(), "Not initialized");
        ensure!(path.join("flake.nix").is_file(), "Not initialized");
        Ok(Self { path })
    }
}

impl Repository {
    fn path_for<T>(&self, location: &'static str, id: Id<T>) -> PathBuf {
        self.path.join(format!("{location}/{id}.toml"))
    }

    #[instrument(ret)]
    fn get<T: for<'a> Deserialize<'a> + Debug>(&self, path: &Path) -> Result<T> {
        Ok(toml::from_str(&fs::read_to_string(path)?)?)
    }

    #[instrument]
    fn set<T: Serialize + Debug>(&mut self, path: &Path, value: &T) -> Result<()> {
        fs::write(path, toml::to_string_pretty(&value)?)?;
        Ok(())
    }

    #[instrument(skip(f))]
    fn modify<T: Serialize + for<'a> Deserialize<'a> + Debug>(
        &mut self,
        path: &Path,
        f: impl FnOnce(&mut T) -> Result<()>,
    ) -> Result<T> {
        let mut value = self.get(path)?;
        f(&mut value)?;
        self.set(path, &value)?;
        Ok(value)
    }
}

impl Repository {
    fn add_to_account<T>(&mut self, acc: Id<Account<T>>, amount: Amount) -> Result<()> {
        let acc: Id<Account> = acc.erase().unerase();
        self.modify(&self.path_for("accounts", acc), |account: &mut Account| {
            debug!(?account, ?amount);
            ensure!(account.current.add(amount).0 >= 0, "Account balance not permitted to be below 0 in any currency");
            Ok(())
        })?;
        Ok(())
    }
    // Only exists to make it more obvious than a single character that transactions are removing, not adding, their amounts to the accounts
    fn draw_from_account<T>(&mut self, acc: Id<Account<T>>, amount: Amount) -> Result<()> {
        self.add_to_account(acc, -amount)
    }

    fn add_transaction(&mut self, transaction: Transaction) -> Result<()> {
        self.set(&self.path_for("transactions", transaction.id), &transaction)?;
        let Transaction { id, amount, .. } = transaction;
        match transaction.inner {
            TransactionInner::Received { src: _, dst, dst_virt } => {
                self.add_to_account(dst, amount)?;
                self.add_to_account(dst_virt, amount)?;
            },
            TransactionInner::Paid { src, src_virt, dst: _ } => {
                self.draw_from_account(src, amount)?;
                self.draw_from_account(src_virt, amount)?;
            },
            TransactionInner::MovePhys { src, dst, fees } => {
                // MovePhys amount is the amount received: fees are added to `amount` *before* drawing them
                self.draw_from_account(src, amount + fees)?;
                self.add_to_account(dst, amount)?;
            },
            TransactionInner::MoveVirt { src, dst } => {
                self.draw_from_account(src, amount)?;
                self.add_to_account(dst, amount)?;
            },
            TransactionInner::Convert {
                acc,
                acc_virt,
                new_amount,
            } => {
                self.draw_from_account(acc, amount)?;
                self.draw_from_account(acc_virt, amount)?;
                // TODO: don't rewrite both files twice?
                self.add_to_account(acc, new_amount)?;
                self.add_to_account(acc_virt, new_amount)?;
            },
        };
        git!(in &self.path, "add", format!("transactions/{}.toml", id))?;
        Ok(())
    }

    #[instrument]
    fn create_account(&mut self, account: Account) -> Result<()> {
        self.set(&self.path_for("accounts", account.id), &account)?;
        git!(in &self.path, "add", format!("accounts/{}.toml", account.id))?;
        Ok(())
    }

    #[instrument]
    fn modify_account(&mut self, id: Id<Account>, changes: Vec<AccountModification>) -> Result<()> {
        self.modify(&self.path_for("accounts", id), |account: &mut Account| {
            for change in changes {
                match change {
                    AccountModification::Disable => {
                        account.enabled = false;
                    }
                    AccountModification::UpdateName(name) => {
                        account.name = name;
                    }
                    AccountModification::UpdateNotes(notes) => {
                        account.notes = notes;
                    }
                }
            }
            Ok(())
        })?;
        Ok(())
    }

    #[instrument]
    pub fn run_command(&mut self, cmd: Command) -> Result<()> {
        let message = format!("{cmd}");
        match cmd {
            Command::CreateAccount(account) => self.create_account(account)?,
            Command::UpdateAccount(id, f) => self.modify_account(id, f)?,
            Command::AddTransaction(transaction) => self.add_transaction(transaction)?,
        }
       
         git!(in &self.path, "commit", "-a", "-m", message)?;
        Ok(())
    }
}

impl Repository {
    #[instrument]
    pub fn transactions(&self) -> Result<Vec<Id<Transaction>>> {
        let mut out = Vec::new();
        for file in self.path.join("transactions").read_dir()? {
            let Ok(filename) = file?.file_name().into_string() else { continue };
            let Some(id) = filename.strip_suffix(".toml") else { continue };
            out.push(id.parse().map_err(|e| eyre!("{e}"))?);
        }
        Ok(out)
    }

    #[instrument]
    pub fn get_transaction(&self, id: Id<Transaction>) -> Result<Transaction> {
        self.get(&self.path_for("transactions", id))
    }

    #[instrument]
    pub fn accounts(&self) -> Result<Vec<Account>> {
        let mut out = Vec::new();
        for file in self.path.join("accounts").read_dir()? {
            let Ok(filename) = file?.file_name().into_string() else { continue };
            let Some(id) = filename.strip_suffix(".toml") else { continue };
            let id = id.parse().map_err(|e| eyre!("{e}"))?;
            out.push(self.get_account(id)?);
        }
        Ok(out)
    }

    #[instrument]
    pub fn get_account(&self, id: Id<Account>) -> Result<Account> {
        self.get(&self.path_for("accounts", id))
    }
}
