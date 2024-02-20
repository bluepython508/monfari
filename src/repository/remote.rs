use eyre::{bail, ensure, eyre, Result};
use serde::{de::DeserializeOwned, Deserialize, Serialize};
use std::{
    env,
    ffi::{OsStr, OsString},
    fmt::{self, Debug},
    io::{stdin, stdout, BufRead, BufReader, BufWriter, Read, Write},
    net::{TcpListener, TcpStream},
    process,
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

struct Connection {
    writer: BufWriter<Box<dyn Write + Send>>,
    reader: BufReader<Box<dyn Read + Send>>,
}
impl fmt::Debug for Connection {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "Connection(_)")
    }
}

impl Connection {
    pub fn new(reader: impl Read + Send + 'static, writer: impl Write + Send + 'static) -> Self {
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
enum RemoteHandle {
    Tcp(Connection),
    Http {
        agent: ureq::Agent,
        base_url: String,
    },
}

impl RemoteHandle {
    #[instrument]
    fn connect_tcp(stream: TcpStream) -> Result<(Self, Vec<Account>)> {
        let mut connection = Connection::new(stream.try_clone()?, stream);
        let accounts = connection.receive()?;
        Ok((Self::Tcp(connection), accounts))
    }

    #[instrument]
    fn connect_http(mut base_url: String) -> Result<(Self, Vec<Account>)> {
        if base_url.ends_with('/') {
            base_url.pop();
        };
        let agent = ureq::Agent::new();
        let accounts = agent.get(&format!("{base_url}/")).call()?.into_json()?;
        Ok((Self::Http { agent, base_url }, accounts))
    }

    #[instrument]
    fn run_command(&mut self, command: Command) -> Result<Vec<Account>> {
        match self {
            Self::Tcp(conn) => {
                conn.send(Message::Command { command })?;
                conn.receive()
            }
            Self::Http { agent, base_url } => Ok(agent
                .post(&format!("{base_url}/"))
                .send_json(command)?
                .into_json()?),
        }
    }

    #[instrument]
    fn transactions(&mut self, account: Id<Account>) -> Result<Vec<Transaction>> {
        match self {
            Self::Tcp(conn) => {
                conn.send(Message::Transactions { account })?;
                conn.receive()
            }
            Self::Http { agent, base_url } => Ok(agent
                .get(&format!("{base_url}/transactions/{account}"))
                .call()?
                .into_json()?),
        }
    }
}

#[derive(Debug)]
pub(super) struct RemoteRepository {
    handle: RemoteHandle,
    accounts: Vec<Account>,
}

impl RemoteRepository {
    #[instrument]
    pub(super) fn open_tcp(stream: TcpStream) -> Result<Self> {
        let (handle, accounts) = RemoteHandle::connect_tcp(stream)?;
        Ok(Self { handle, accounts })
    }

    #[instrument]
    pub(super) fn open_http(url: String) -> Result<Self> {
        let (handle, accounts) = RemoteHandle::connect_http(url)?;
        Ok(Self { handle, accounts })
    }
}

impl RemoteRepository {
    #[instrument]
    pub(super) fn run_command(&mut self, command: Command) -> Result<()> {
        self.accounts = self.handle.run_command(command)?;
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
        self.handle.transactions(account)
    }
}

#[instrument]
fn run_session(mut connection: Connection, repo: &OsStr) -> Result<()> {
    let mut repo = Repository::open(repo)?;
    connection.send(repo.accounts()?)?;
    while let Some(msg) = connection.receive_or_eof::<Message>()? {
        debug!(?msg);
        match msg {
            Message::Command { command } => {
                repo.run_command(command)?;
                connection.send(repo.accounts()?)?;
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
#[cfg(unix)]
mod systemd {
    use super::*;
    use std::os::unix::io::FromRawFd;

    #[instrument]
    fn is_fd_inet_socket(fd: i32) -> Result<bool> {
        use nix::sys::socket::{getsockname, AddressFamily::*, SockaddrLike, SockaddrStorage};
        Ok(getsockname::<SockaddrStorage>(fd)?
            .family()
            .is_some_and(|f| matches!(f, Inet | Inet6)))
    }

    #[instrument]
    pub fn serve_systemd_listener(repo: OsString) -> Result<()> {
        ensure!(
            env::var("LISTEN_PID")?.parse::<u32>()? == process::id(),
            "This process should not be listening for systemd sockets"
        );
        let n_fds = env::var("LISTEN_FDS")?.parse::<i32>()?;
        let mut listeners = (3..3 + n_fds)
            .map(|fd| {
                ensure!(
                    is_fd_inet_socket(fd)?,
                    "Systemd-provided fd is not an inet socket!"
                );
                Ok(unsafe { TcpListener::from_raw_fd(fd) })
            })
            .collect::<Result<Vec<_>>>()?;
        let Some(listener) = listeners.pop() else { bail!("One listener must be provided") };
        ensure!(
            listeners.is_empty(),
            "More than one listener is not supported at present"
        );
        serve_listener(listener, repo)
    }
}

mod http {
    use tiny_http::{Header, Method, Request, Response};
    use tracing::info_span;

    use super::*;

    fn json(r: Request, s: impl Serialize) -> Result<()> {
        let body = serde_json::to_string(&s)?;
        r.respond(
            Response::from_string(body)
                .with_status_code(200)
                .with_header(
                    Header::from_bytes(&b"Content-Type"[..], &b"application/json"[..]).unwrap(),
                ),
        )?;
        Ok(())
    }
    fn err(request: Request, code: u32, reason: &'static str) -> Result<()> {
        request.respond(Response::from_string(reason).with_status_code(code))?;
        Ok(())
    }

    #[instrument]
    pub fn serve_http(addr: String, repo: OsString) -> Result<()> {
        let mut repo = Repository::open(&repo)?;

        let server = tiny_http::Server::http(addr).map_err(|e| eyre!(e))?;
        for mut request in server.incoming_requests() {
            let _span =
                info_span!("request", url = request.url(), method = ?request.method()).entered();
            match (
                request.method(),
                &request.url().split('/').skip(1).collect::<Vec<&str>>()[..],
            ) {
                (&Method::Get, &[""]) => json(request, &repo.accounts()?)?,
                (&Method::Post, &[""]) => {
                    let Some("application/json") = request.headers().iter().rev().find(|x| x.field.equiv("Content-Type")).map(|x| x.value.as_str()) else { err(request, 401, "JSON is required")?; continue };
                    let Ok(command) = serde_json::from_reader(request.as_reader()) else { err(request, 401, "Invalid command")?; continue };
                    repo.run_command(command)?;
                    json(request, repo.accounts()?)?
                }
                (&Method::Get, &["transactions", account]) => {
                    let Ok(account) = account.parse() else { err(request, 401, "Invalid account ID")?; continue };
                    json(request, &repo.transactions(account)?)?
                }
                (&Method::Post, &["__stop__"]) => break,
                _ => err(request, 404, "Not Found")?,
            };
        }
        Ok(())
    }
}

#[instrument]
pub fn serve(mode: crate::ServeMode, repo: OsString) -> Result<()> {
    match mode {
        crate::ServeMode::Stdio => run_session(Connection::new(stdin(), stdout()), &repo),
        crate::ServeMode::Bind { addr } => serve_listener(TcpListener::bind(addr)?, repo),
        crate::ServeMode::Http { addr } => http::serve_http(addr, repo),
        #[cfg(unix)]
        crate::ServeMode::Systemd => systemd::serve_systemd_listener(repo),
    }
}
