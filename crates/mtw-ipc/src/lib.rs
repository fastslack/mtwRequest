//! Local IPC for mtwRequest.
//!
//! One endpoint string per link. A value that starts with `\\.\pipe\` is a
//! Windows named pipe; anything else is a Unix domain socket path. Each kind is
//! only valid on its own platform, and using the other one is a configuration
//! error (`InvalidInput`), never a silent fallback.

use std::io;
use std::pin::Pin;
use std::task::{Context, Poll};

use tokio::io::{AsyncRead, AsyncWrite, ReadBuf};

/// Prefix of a Windows named-pipe endpoint.
pub const PIPE_PREFIX: &str = r"\\.\pipe\";

/// Whether `endpoint` names a Windows named pipe.
pub fn is_pipe(endpoint: &str) -> bool {
    endpoint.starts_with(PIPE_PREFIX)
}

fn check_platform(endpoint: &str) -> io::Result<()> {
    #[cfg(windows)]
    if !is_pipe(endpoint) {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            format!("on Windows the endpoint must be a named pipe ({PIPE_PREFIX}...), got '{endpoint}'"),
        ));
    }
    #[cfg(not(windows))]
    if is_pipe(endpoint) {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            format!("named pipe endpoint '{endpoint}' is only valid on Windows"),
        ));
    }
    Ok(())
}

/// A connected local stream: either side of a Unix socket or a named pipe.
pub enum IpcStream {
    #[cfg(unix)]
    Unix(tokio::net::UnixStream),
    #[cfg(windows)]
    PipeClient(tokio::net::windows::named_pipe::NamedPipeClient),
    #[cfg(windows)]
    PipeServer(tokio::net::windows::named_pipe::NamedPipeServer),
}

macro_rules! delegate {
    ($self:ident, $s:ident => $e:expr) => {
        match $self.get_mut() {
            #[cfg(unix)]
            IpcStream::Unix($s) => $e,
            #[cfg(windows)]
            IpcStream::PipeClient($s) => $e,
            #[cfg(windows)]
            IpcStream::PipeServer($s) => $e,
        }
    };
}

impl AsyncRead for IpcStream {
    fn poll_read(self: Pin<&mut Self>, cx: &mut Context<'_>, buf: &mut ReadBuf<'_>) -> Poll<io::Result<()>> {
        delegate!(self, s => Pin::new(s).poll_read(cx, buf))
    }
}

impl AsyncWrite for IpcStream {
    fn poll_write(self: Pin<&mut Self>, cx: &mut Context<'_>, buf: &[u8]) -> Poll<io::Result<usize>> {
        delegate!(self, s => Pin::new(s).poll_write(cx, buf))
    }
    fn poll_flush(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        delegate!(self, s => Pin::new(s).poll_flush(cx))
    }
    fn poll_shutdown(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        delegate!(self, s => Pin::new(s).poll_shutdown(cx))
    }
}

/// Connect to `endpoint`.
///
/// On Windows a pipe whose every instance is momentarily taken answers
/// `ERROR_PIPE_BUSY`; that is retried for up to 5 s. "Not there at all" is
/// returned at once so callers keep their own backoff.
pub async fn connect(endpoint: &str) -> io::Result<IpcStream> {
    check_platform(endpoint)?;
    #[cfg(unix)]
    {
        Ok(IpcStream::Unix(tokio::net::UnixStream::connect(endpoint).await?))
    }
    #[cfg(windows)]
    {
        use std::time::Duration;
        use tokio::net::windows::named_pipe::ClientOptions;
        const ERROR_PIPE_BUSY: i32 = 231;
        let deadline = tokio::time::Instant::now() + Duration::from_secs(5);
        loop {
            match ClientOptions::new().open(endpoint) {
                Ok(c) => return Ok(IpcStream::PipeClient(c)),
                Err(e) if e.raw_os_error() == Some(ERROR_PIPE_BUSY) && tokio::time::Instant::now() < deadline => {
                    tokio::time::sleep(Duration::from_millis(50)).await;
                }
                Err(e) => return Err(e),
            }
        }
    }
}

/// Listens on a local endpoint.
pub struct IpcListener {
    endpoint: String,
    #[cfg(unix)]
    inner: tokio::net::UnixListener,
    /// The instance waiting for the next client. A new one is created before
    /// a connected one is handed out, so the pipe name never disappears
    /// between clients (a reconnecting peer would otherwise see "not found").
    #[cfg(windows)]
    next: tokio::net::windows::named_pipe::NamedPipeServer,
}

impl IpcListener {
    /// Bind `endpoint`. Unix: creates the parent dir and replaces a stale
    /// socket file. Windows: creates the first pipe instance, refusing remote
    /// clients and a name some other process already owns.
    /// Must be called inside a tokio runtime.
    pub fn bind(endpoint: &str) -> io::Result<Self> {
        check_platform(endpoint)?;
        #[cfg(unix)]
        {
            if let Some(dir) = std::path::Path::new(endpoint).parent() {
                if !dir.as_os_str().is_empty() {
                    std::fs::create_dir_all(dir)?;
                }
            }
            let _ = std::fs::remove_file(endpoint);
            let inner = tokio::net::UnixListener::bind(endpoint)?;
            Ok(Self { endpoint: endpoint.to_string(), inner })
        }
        #[cfg(windows)]
        {
            use tokio::net::windows::named_pipe::ServerOptions;
            let next = ServerOptions::new()
                .first_pipe_instance(true)
                .reject_remote_clients(true)
                .create(endpoint)?;
            Ok(Self { endpoint: endpoint.to_string(), next })
        }
    }

    /// Wait for the next client.
    pub async fn accept(&mut self) -> io::Result<IpcStream> {
        #[cfg(unix)]
        {
            let (s, _) = self.inner.accept().await?;
            Ok(IpcStream::Unix(s))
        }
        #[cfg(windows)]
        {
            use tokio::net::windows::named_pipe::ServerOptions;
            self.next.connect().await?;
            let fresh = ServerOptions::new().reject_remote_clients(true).create(&self.endpoint)?;
            let connected = std::mem::replace(&mut self.next, fresh);
            Ok(IpcStream::PipeServer(connected))
        }
    }

    /// The endpoint this listener was bound to.
    pub fn endpoint(&self) -> &str {
        &self.endpoint
    }
}

/// A fresh endpoint for tests: a socket file inside `dir` on Unix, a pipe
/// name unique to this process and call on Windows.
pub fn unique_test_endpoint(dir: &std::path::Path, name: &str) -> String {
    #[cfg(unix)]
    {
        dir.join(format!("{name}.sock")).to_string_lossy().to_string()
    }
    #[cfg(windows)]
    {
        use std::sync::atomic::{AtomicUsize, Ordering};
        static N: AtomicUsize = AtomicUsize::new(0);
        let _ = dir;
        format!("{PIPE_PREFIX}mtw-test-{name}-{}-{}", std::process::id(), N.fetch_add(1, Ordering::Relaxed))
    }
}
