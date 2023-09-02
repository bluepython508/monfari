use std::{fmt::Debug, net::TcpStream, path::PathBuf, sync::Mutex};

use eyre::Result;
use tracing::instrument;

use crate::{command::*, types::*};

mod local;
use local::LocalRepository;

mod remote;
use remote::RemoteRepository;

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
    pub fn open(path: PathBuf) -> Result<Self> {
        Ok(Self(RepositoryInner::Local(LocalRepository::open(path)?)))
    }

    #[instrument]
    pub fn connect(stream: TcpStream) -> Result<Self> {
        Ok(Self(RepositoryInner::Remote(Mutex::new(
            RemoteRepository::connect(stream)?,
        ))))
    }

    pub fn run_command(&mut self, cmd: Command) -> Result<()> {
        match &mut self.0 {
            RepositoryInner::Local(repo) => repo.run_command(cmd),
            RepositoryInner::Remote(repo) => repo.get_mut().unwrap().run_command(cmd),
        }
    }

    pub fn accounts(&self) -> Result<Vec<Account>> {
        match &self.0 {
            RepositoryInner::Local(repo) => repo.accounts(),
            RepositoryInner::Remote(repo) => repo.lock().unwrap().accounts(),
        }
    }

    pub fn account(&self, id: Id<Account>) -> Result<Account> {
        match &self.0 {
            RepositoryInner::Local(repo) => repo.account(id),
            RepositoryInner::Remote(repo) => repo.lock().unwrap().account(id),
        }
    }
}

pub use remote::serve;
