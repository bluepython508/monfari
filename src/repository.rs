use std::{
    fmt::Debug,
    fs,
    io::Write,
    path::PathBuf,
    process::{self, Stdio},
};

use eyre::{ensure, eyre, Result, WrapErr};
use itertools::Itertools;
use serde::{de::DeserializeOwned, Serialize};
use tracing::{debug, instrument};

use crate::{command::*, types::*};

#[instrument]
fn cmd(cmd: &mut process::Command) -> Result<()> {
    let status = cmd
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()?;
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
    fn release(self) {}
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
pub struct Repository {
    path: PathBuf,
    lock: LockFile,
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
        if path.try_exists()? {
            ensure!(
                path.read_dir()?.next().is_none(),
                "Path must be an empty or non-existent directory"
            );
        } else {
            fs::create_dir_all(&path)?;
        }
        fs::write(path.join("flake.nix"), FLAKE_NIX_TMPL)?;
        fs::write(path.join(".gitignore"), "monfari-repo-lock\n")?;

        for dir in ["transactions", "accounts"] {
            let p = path.join(dir);
            fs::create_dir_all(&p)?;
            fs::File::create(p.join(".gitkeep"))?;
        }

        git!(in &path, "init")?;
        git!(in &path, "add", "transactions", "accounts", ".gitignore", "flake.nix")?;
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

        let lock = LockFile::acquire(path.join("monfari-repo-lock"))?;
        let mut this = Self { path, lock };
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
        git!(in &path, "status").wrap_err("Not initialized")?;
        git!(in &path, "diff-index", "--quiet", "HEAD")
            .wrap_err("repo is dirty - monfari has crashed previously")?;
        ensure!(path.join("accounts").is_dir(), "Not initialized");
        ensure!(path.join("transactions").is_dir(), "Not initialized");
        ensure!(path.join("flake.nix").is_file(), "Not initialized");
        let lock = LockFile::acquire(path.join("monfari-repo-lock"))?;
        Ok(Self { path, lock })
    }
}

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

impl Repository {
    fn path_for<T: Entity>(&self, id: Id<T>) -> PathBuf {
        self.path.join(format!("{}/{id}.toml", T::PATH))
    }

    #[instrument(ret)]
    pub fn get<T: Entity>(&self, id: Id<T>) -> Result<T> {
        Ok(toml::from_str(&fs::read_to_string(self.path_for(id))?)?)
    }

    #[instrument]
    fn set<T: Entity>(&mut self, value: &T) -> Result<()> {
        let path = self.path_for(value.id());
        fs::write(&path, toml::to_string_pretty(&value)?)?;
        git!(in &self.path, "add", &path)?;
        Ok(())
    }

    #[instrument(skip(f))]
    fn modify<T: Entity>(&mut self, id: Id<T>, f: impl FnOnce(&mut T) -> Result<()>) -> Result<T> {
        let mut value = self.get(id)?;
        f(&mut value)?;
        assert!(value.id() == id);
        self.set(&value)?;
        Ok(value)
    }
}

impl Repository {
    #[instrument]
    fn add_transaction(&mut self, transaction: Transaction) -> Result<()> {
        self.set(&transaction)?;
        for (acc, amounts) in &transaction.results().into_iter().group_by(|x| x.0) {
            self.modify(acc, |acc| {
                for amount in amounts {
                    acc.current += amount.1;
                }
                ensure!(
                    acc.current.0.values().all(|x| x.0 >= 0),
                    "Account balance must never be below 0 in any currency"
                );
                Ok(())
            })?;
        }
        Ok(())
    }

    #[instrument]
    fn create_account(&mut self, account: Account) -> Result<()> {
        self.set(&account)?;
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

        git!(in &self.path, "commit", "-m", message)?;
        Ok(())
    }
}

impl Repository {
    #[instrument]
    pub fn list<T: Entity>(&self) -> Result<Vec<Id<T>>> {
        self.path
            .join(T::PATH)
            .read_dir()?
            .filter_map_ok(|entry| entry.file_name().into_string().ok())
            .filter_map_ok(|filename| Some(filename.strip_suffix(".toml")?.to_owned()))
            .map(|x| x?.parse::<Id<T>>().map_err(|e| eyre!("{e}")))
            .collect()
    }
}
