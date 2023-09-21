use eyre::{ensure, eyre, Context, Result, bail};
use serde::{de::DeserializeOwned, Serialize, Deserialize};
use std::{
    env,
    ffi::{OsStr, OsString},
    fmt::{self, Debug},
    io::{stdin, stdout, BufRead, BufReader, BufWriter, Read, Write},
    net::TcpListener,
    process, os::fd::FromRawFd,
};
use tracing::{debug, instrument};

use crate::command::Command;
use crate::types::*;

use super::Repository;

#[derive(Serialize, Deserialize, Debug, Clone)]
enum Message {
    Command { command: Command },
    Transactions { account: Id<Account> },
}

pub struct Connection {
    writer: BufWriter<Box<dyn Write + Send>>,
    reader: BufReader<Box<dyn Read + Send>>,
}
impl fmt::Debug for Connection {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "Connection(_)")
    }
}

impl Connection {
    pub fn new(
        reader: impl Read + Send + 'static,
        writer: impl Write + Send + 'static,
    ) -> Self {
        Self {
            writer: BufWriter::new(Box::new(writer)),
            reader: BufReader::new(Box::new(reader)),
        }
    }

    #[instrument]
    fn send<T: Serialize + Debug>(&mut self, message: T) -> Result<()> {
        serde_json::to_writer(&mut self.writer, &message)?;
        self.writer.write_all(&[0])?;
        self.writer.flush()?;
        Ok(())
    }

    #[instrument(ret)]
    fn receive<T: DeserializeOwned + Debug>(&mut self) -> Result<T> {
        self.receive_or_eof()
            .and_then(|x| x.ok_or_else(|| eyre!("Unexpected EOF")))
    }

    #[instrument(ret)]
    fn receive_or_eof<T: DeserializeOwned + Debug>(&mut self) -> Result<Option<T>> {
        if self.reader.fill_buf()?.is_empty() {
            return Ok(None);
        } // EOF
        let mut buf = vec![];
        self.reader.read_until(0, &mut buf)?;
        buf.pop(); // Should always have a NUL suffix, as send will always add one. read_until includes it if it's present before EOF
        debug!(str = ?std::str::from_utf8(&buf));
        Ok(Some(serde_json::from_slice(&buf)?))
    }
}

#[derive(Debug)]
pub(super) struct RemoteRepository {
    connection: Connection,
    accounts: Vec<Account>,
}

impl RemoteRepository {
    #[instrument]
    pub(super) fn open(mut connection: Connection) -> Result<Self> {
        Ok(Self {
            accounts: connection.receive()?,
            connection,
        })
    }
}

impl RemoteRepository {
    #[instrument]
    pub(super) fn run_command(&mut self, command: Command) -> Result<()> {
        self.connection.send(Message::Command { command })?;
        self.accounts = self
            .connection
            .receive()
            .wrap_err("Expected to receive accounts list after command")?;
        Ok(())
    }

    #[instrument]
    pub(super) fn accounts(&mut self) -> Vec<Account> {
        self.accounts.clone()
    }

    #[instrument]
    pub(super) fn account(&mut self, id: Id<Account>) -> Option<Account> {
        self.accounts.iter().find(|x| x.id == id).cloned()
    }

    #[instrument]
    pub(super) fn transactions(&mut self, account: Id<Account>) -> Result<Vec<Transaction>> {
        self.connection.send(Message::Transactions { account })?;
        self.connection.receive()
    }
}

#[instrument]
fn run_session(mut connection: Connection, repo: &OsStr) -> Result<()> {
    let mut repo = Repository::open(repo)?;
    connection.send(repo.accounts())?;
    while let Some(msg) = connection.receive_or_eof::<Message>()? {
        debug!(?msg);
        match msg {
            Message::Command { command } => {
                repo.run_command(command)?;
                connection.send(repo.accounts())?;
            }
            Message::Transactions { account } => {
                connection.send(repo.transactions(account)?)?;
            }
        }
    }
    Ok(())
}

#[instrument]
fn serve_listener(listener: TcpListener, repo: OsString) -> Result<()> {
    loop {
        let (stream, _) = listener.accept()?;
        let connection = Connection::new(BufReader::new(stream.try_clone()?), stream);
        run_session(connection, &repo)?;
    }
}

#[instrument]
fn is_fd_inet_socket(fd: i32) -> Result<bool> {
    use nix::sys::socket::{getsockname, AddressFamily::*, SockaddrLike, SockaddrStorage};
    Ok(getsockname::<SockaddrStorage>(fd)?
        .family()
        .is_some_and(|f| matches!(f, Inet | Inet6)))
}

#[instrument]
fn serve_systemd_listener(repo: OsString) -> Result<()> {
    ensure!(
        env::var("LISTEN_PID")?.parse::<u32>()? == process::id(),
        "This process should not be listening for systemd sockets"
    );
    let n_fds = env::var("LISTEN_FDS")?.parse::<i32>()?;
    let mut listeners = (3..3 + n_fds).map(|fd| {
        ensure!(is_fd_inet_socket(fd)?, "Systemd-provided fd is not an inet socket!");
        Ok(unsafe { TcpListener::from_raw_fd(fd) })
    }).collect::<Result<Vec<_>>>()?;
    let Some(listener) = listeners.pop() else { bail!("One listener must be provided") };
    ensure!(listeners.is_empty(), "More than one listener is not supported at present");
    serve_listener(listener, repo)
}

#[instrument]
pub fn serve(mode: crate::ServeMode, repo: OsString) -> Result<()> {
    match mode {
        crate::ServeMode::Stdio => run_session(Connection::new(stdin(), stdout()), &repo),
        crate::ServeMode::Bind { addr } => serve_listener(TcpListener::bind(addr)?, repo),
        crate::ServeMode::Systemd => serve_systemd_listener(repo),
    }
}
