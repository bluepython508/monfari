use std::{
    ffi::OsStr,
    fmt::Debug,
    net::{TcpStream, ToSocketAddrs},
    path::{Path, PathBuf},
    sync::Mutex,
};

use eyre::{bail, Result};
use tracing::instrument;

use crate::{command::*, types::*};

mod local;
use local::LocalRepository;

mod remote;
use remote::RemoteRepository;

pub use remote::serve;

#[derive(Debug)]
enum RepositoryInner {
    Local(LocalRepository),
    Remote(Mutex<RemoteRepository>),
}

#[derive(Debug)]
pub struct Repository(RepositoryInner);

impl Repository {
    #[instrument]
    pub fn init(path: PathBuf) -> Result<Self> {
        Ok(Self(RepositoryInner::Local(LocalRepository::init(path)?)))
    }

    #[instrument]
    pub fn open(addr: &OsStr) -> Result<Repository> {
        let Some(addr) = addr.to_str() else { return Self::open_local(addr.as_ref()) };
        match addr.split_once(':') {
            None => Self::open_local(addr.as_ref()),
            Some(("path", path)) => Self::open_local(path.as_ref()),
            Some(("tcp", addr)) => Self::open_tcp(addr),
            Some(("http" | "https", _)) => Self::open_http(addr.to_owned()),
            Some((proto, _)) => bail!("Unknown proto {proto}"),
        }
    }

    fn open_local(path: &Path) -> Result<Self> {
        Ok(Self(RepositoryInner::Local(LocalRepository::open(
            path.to_owned(),
        )?)))
    }

    fn open_tcp(s: impl ToSocketAddrs) -> Result<Self> {
        let stream = TcpStream::connect(s)?;
        Ok(Self(RepositoryInner::Remote(Mutex::new(
            RemoteRepository::open_tcp(stream)?,
        ))))
    }

    fn open_http(s: String) -> Result<Self> {
        Ok(Self(RepositoryInner::Remote(Mutex::new(
            RemoteRepository::open_http(s)?,
        ))))
    }

    pub fn run_command(&mut self, cmd: Command) -> Result<()> {
        match &mut self.0 {
            RepositoryInner::Local(repo) => repo.run_command(cmd),
            RepositoryInner::Remote(repo) => repo.get_mut().unwrap().run_command(cmd),
        }
    }

    pub fn accounts(&self) -> Vec<Account> {
        match &self.0 {
            RepositoryInner::Local(repo) => repo.accounts(),
            RepositoryInner::Remote(repo) => repo.lock().unwrap().accounts(),
        }
    }

    pub fn account(&self, id: Id<Account>) -> Option<Account> {
        match &self.0 {
            RepositoryInner::Local(repo) => repo.account(id),
            RepositoryInner::Remote(repo) => repo.lock().unwrap().account(id),
        }
    }

    pub fn transactions(&self, id: Id<Account>) -> Result<Vec<Transaction>> {
        match &self.0 {
            RepositoryInner::Local(repo) => repo.transactions(id),
            RepositoryInner::Remote(repo) => repo.lock().unwrap().transactions(id),
        }
    }
}
