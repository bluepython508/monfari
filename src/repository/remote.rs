use eyre::{eyre, Context, Result};
use serde::{de::DeserializeOwned, Serialize};
use tracing::{instrument, debug};
use std::{
    fmt,
    io::{BufRead, BufReader, BufWriter, Write},
    net::{SocketAddr, TcpStream, TcpListener}, ffi::OsString
};

use crate::command::Command;
use crate::types::*;

use super::Repository;

struct Connection {
    writer: BufWriter<Box<dyn Write + Send>>,
    reader: Box<dyn BufRead + Send>,
}
impl fmt::Debug for Connection {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "Connection(_)")
    }
}

impl Connection {
    pub fn new(reader: impl BufRead + Send + 'static, writer: impl Write + Send + 'static) -> Result<Self> {
        Ok(Self {
            writer: BufWriter::new(Box::new(writer)),
            reader: Box::new(reader),
        })
    }

    fn send<T: Serialize>(&mut self, message: T) -> Result<()> {
        serde_json::to_writer(&mut self.writer, &message)?;
        self.writer.write_all(&[0])?;
        self.writer.flush()?;
        Ok(())
    }

    fn receive<T: DeserializeOwned>(&mut self) -> Result<T> {
        self.receive_or_eof()
            .and_then(|x| x.ok_or_else(|| eyre!("Unexpected EOF")))
    }

    fn receive_or_eof<T: DeserializeOwned>(&mut self) -> Result<Option<T>> {
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
    accounts: Vec<Account>
}

impl RemoteRepository {
    #[instrument]
    pub(super) fn connect(stream: TcpStream) -> Result<Self> {
        let mut connection = Connection::new(BufReader::new(stream.try_clone()?), stream)?;
        Ok(Self {
            accounts: connection.receive()?,
            connection,
        })
    }
}

impl RemoteRepository {
    #[instrument]
    pub(super) fn run_command(&mut self, command: Command) -> Result<()> {
        self.connection.send(command)?;
        self.accounts = self.connection
            .receive()
            .wrap_err("Expected acknowledgement of command")?;
        Ok(())
    }

    #[instrument]
    pub(super) fn accounts(&mut self) -> Result<Vec<Account>> {
        Ok(self.accounts.clone())
    }

    #[instrument]
    pub(super) fn account(&mut self, id: Id<Account>) -> Result<Account> {
        self.accounts.iter().find(|x| x.id == id).cloned().ok_or_else(|| eyre!("No account with id {id}"))
    }
}

#[instrument]
fn run_session(mut connection: Connection, repo: &mut Repository) -> Result<()> {
    connection.send(repo.accounts()?)?;
    while let Some(msg) = connection.receive_or_eof::<Command>()? {
        debug!(?msg);
        repo.run_command(msg)?;
        connection.send(repo.accounts()?)?;
    };
    Ok(())
}

#[instrument]
pub fn serve(addr: SocketAddr, repo: OsString) -> Result<()> {
    let listener = TcpListener::bind(addr)?;
    loop {
        let (stream, _) = listener.accept()?;
        let connection = Connection::new(BufReader::new(stream.try_clone()?), stream)?;
        let mut repo = crate::open(repo.clone())?;
        run_session(connection, &mut repo)?;
    }
}
