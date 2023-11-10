use std::{collections::BTreeMap, fmt::Debug, fs, io::Write, path::PathBuf, process};

use eyre::{ensure, eyre, Context, Result};
use itertools::Itertools;
use serde::{de::DeserializeOwned, Serialize};
use tracing::{debug, instrument};

use crate::{command::*, types::*};

pub trait Entity: DeserializeOwned + Serialize + Debug {
    const PATH: &'static str;
    fn id(&self) -> Id<Self>;
}
impl Entity for Account {
    const PATH: &'static str = "accounts";
    fn id(&self) -> Id<Self> {
        self.id
    }
}
impl Entity for Transaction {
    const PATH: &'static str = "transactions";
    fn id(&self) -> Id<Self> {
        self.id
    }
}

#[instrument]
fn cmd(cmd: &mut process::Command) -> Result<String> {
    let output = cmd.output()?;
    debug!(?output);
    ensure!(
        output.status.success(),
        "Command {cmd:?} did not exist successfully
            stderr: {:?}
            stdout: {:?}
        ",
        String::from_utf8_lossy(&output.stderr),
        String::from_utf8_lossy(&output.stdout)
    );
    Ok(String::from_utf8(output.stdout)?)
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
struct LockFile(fs::File, PathBuf);

impl LockFile {
    fn acquire(path: PathBuf) -> Result<Self> {
        let mut f = fs::File::options()
            .create_new(true)
            .write(true)
            .open(&path)
            .wrap_err("Repo is locked by another process")?;
        write!(f, "{}", std::process::id())?;
        Ok(Self(f, path))
    }
}

impl Drop for LockFile {
    fn drop(&mut self) {
        assert_eq!(
            fs::read_to_string(&self.1).unwrap(),
            std::process::id().to_string()
        );
        fs::remove_file(&self.1).unwrap();
    }
}

#[derive(Debug)]
pub(super) struct LocalRepository {
    path: PathBuf,
    _lock: LockFile,
    accounts: BTreeMap<Id<Account>, Account>,
}

impl LocalRepository {
    #[instrument]
    pub(super) fn init(path: PathBuf) -> Result<Self> {
        if path.try_exists()? {
            ensure!(
                path.read_dir()?.next().is_none(),
                "Path must be an empty or non-existent directory"
            );
        } else {
            fs::create_dir_all(&path)?;
        }
        fs::write(path.join(".gitignore"), "monfari-repo-lock\n")?;

        for dir in ["transactions", "accounts"] {
            let p = path.join(dir);
            fs::create_dir_all(&p)?;
            fs::File::create(p.join(".gitkeep"))?;
        }

        git!(in &path, "init")?;
        git!(in &path, "add", "transactions", "accounts", ".gitignore")?;

        let lock = LockFile::acquire(path.join("monfari-repo-lock"))?;
        let mut this = Self {
            path,
            _lock: lock,
            accounts: Default::default(),
        };
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
    pub(super) fn open(path: PathBuf) -> Result<Self> {
        git!(in &path, "status").wrap_err("Not initialized")?;
        git!(in &path, "diff-index", "--quiet", "HEAD")
            .wrap_err("repo is dirty - monfari has crashed previously")?;
        ensure!(path.join("accounts").is_dir(), "Not initialized");
        ensure!(path.join("transactions").is_dir(), "Not initialized");
        let lock = LockFile::acquire(path.join("monfari-repo-lock"))?;
        let mut this = Self {
            path,
            _lock: lock,
            accounts: Default::default(),
        };
        this.accounts = this
            .list::<Account>()?
            .into_iter()
            .map(|acc| Ok((acc, this.get(acc)?)))
            .collect::<Result<_>>()?;
        Ok(this)
    }
}

impl LocalRepository {
    fn path_for<T: Entity>(&self, id: Id<T>) -> PathBuf {
        self.path.join(format!("{}/{id}.toml", T::PATH))
    }

    #[instrument]
    fn create<T: Entity>(&mut self, value: &T) -> Result<()> {
        let path = self.path_for(value.id());
        fs::write(&path, toml::to_string_pretty(&value)?)?;
        git!(in &self.path, "add", &path)?;
        Ok(())
    }

    #[instrument(skip(f))]
    fn modify(
        &mut self,
        id: Id<Account>,
        f: impl FnOnce(&mut Account) -> Result<()>,
    ) -> Result<impl FnOnce(&mut Self) -> Result<()>> {
        let path = self.path_for(id);
        let mut value = self
            .accounts
            .get(&id)
            .ok_or_else(|| eyre!("No such account {id}"))?
            .clone();
        f(&mut value)?;
        assert!(value.id == id);
        Ok(move |repo: &mut Self| {
            let value_r = repo.accounts.get_mut(&id).unwrap();
            *value_r = value;
            fs::write(&path, toml::to_string_pretty(value_r)?)?;
            git!(in &repo.path, "add", &path)?;
            Ok(())
        })
    }
}

impl LocalRepository {
    #[instrument]
    fn add_transaction(&mut self, transaction: Transaction) -> Result<()> {
        self.create(&transaction)?;
        transaction
            .results()
            .into_iter()
            .group_by(|x| x.0)
            .into_iter()
            .map(|(acc, amounts)| {
                self.modify(acc, |acc| {
                    for amount in amounts {
                        acc.current += amount.1;
                    }
                    ensure!(
                        acc.current.0.values().all(|x| x.0 >= 0),
                        "Account balance must never be below 0 in any currency"
                    );
                    Ok(())
                })
            })
            .collect::<Result<Vec<_>>>()?
            .into_iter()
            .try_for_each(|exec| exec(self))?;
        Ok(())
    }

    #[instrument]
    fn create_account(&mut self, account: Account) -> Result<()> {
        self.create(&account)?;
        let id = account.id;
        ensure!(
            self.accounts.insert(id, account).is_none(),
            "Cannot overwrite account with duplicate id {id}"
        );
        Ok(())
    }

    #[instrument]
    fn modify_account(&mut self, id: Id<Account>, changes: Vec<AccountModification>) -> Result<()> {
        self.modify(id, |account| {
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
        })?(self)?;
        Ok(())
    }

    #[instrument]
    fn list<T: Entity>(&self) -> Result<Vec<Id<T>>> {
        self.path
            .join(T::PATH)
            .read_dir()?
            .filter_map_ok(|entry| entry.file_name().into_string().ok())
            .filter_map_ok(|filename| Some(filename.strip_suffix(".toml")?.to_owned()))
            .map(|x| x?.parse::<Id<T>>().map_err(|e| eyre!("{e}")))
            .collect()
    }

    #[instrument(ret)]
    fn get<T: Entity>(&self, id: Id<T>) -> Result<T> {
        Ok(toml::from_str(&fs::read_to_string(self.path_for(id))?)?)
    }
}

impl LocalRepository {
    #[instrument]
    pub(super) fn run_command(&mut self, cmd: Command) -> Result<()> {
        let message = format!("{cmd}");
        match cmd {
            Command::CreateAccount(account) => self.create_account(account)?,
            Command::UpdateAccount(id, f) => self.modify_account(id, f)?,
            Command::AddTransaction(transaction) => self.add_transaction(transaction)?,
        }

        git!(in &self.path, "commit", "-m", message)?;
        Ok(())
    }

    #[instrument]
    pub(super) fn accounts(&self) -> Vec<Account> {
        self.accounts.values().cloned().collect()
    }

    #[instrument]
    pub(super) fn account(&self, id: Id<Account>) -> Option<Account> {
        self.accounts.get(&id).cloned()
    }

    #[instrument]
    pub(super) fn transactions(&self, id: Id<Account>) -> Result<Vec<Transaction>> {
        self.list::<Transaction>()?
            .into_iter()
            .map(|x| self.get(x))
            .filter_ok(|x| x.accounts().contains(&id))
            .collect::<Result<Vec<_>>>()
            .map(|mut x| {
                x.sort_unstable_by_key(|t| t.id);
                x
            })
    }
}
