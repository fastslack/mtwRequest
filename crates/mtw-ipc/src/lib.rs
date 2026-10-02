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
            format!(
                "on Windows the endpoint must be a named pipe ({PIPE_PREFIX}...), got '{endpoint}'"
            ),
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
    fn poll_read(
        self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &mut ReadBuf<'_>,
    ) -> Poll<io::Result<()>> {
        delegate!(self, s => Pin::new(s).poll_read(cx, buf))
    }
}

impl AsyncWrite for IpcStream {
    fn poll_write(
        self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &[u8],
    ) -> Poll<io::Result<usize>> {
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
        Ok(IpcStream::Unix(
            tokio::net::UnixStream::connect(endpoint).await?,
        ))
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
                Err(e)
                    if e.raw_os_error() == Some(ERROR_PIPE_BUSY)
                        && tokio::time::Instant::now() < deadline =>
                {
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
            Ok(Self {
                endpoint: endpoint.to_string(),
                inner,
            })
        }
        #[cfg(windows)]
        {
            let next = pipe_security::create_instance(endpoint, true)?;
            Ok(Self {
                endpoint: endpoint.to_string(),
                next,
            })
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
            if let Err(e) = self.next.connect().await {
                // A client that connects and closes before ConnectNamedPipe
                // completes leaves this instance unusable (ERROR_NO_DATA on
                // every later call). Replace it so the next accept can work.
                if let Ok(fresh) = pipe_security::create_instance(&self.endpoint, false) {
                    self.next = fresh;
                }
                return Err(e);
            }
            let fresh = pipe_security::create_instance(&self.endpoint, false)?;
            let connected = std::mem::replace(&mut self.next, fresh);
            Ok(IpcStream::PipeServer(connected))
        }
    }

    /// The endpoint this listener was bound to.
    pub fn endpoint(&self) -> &str {
        &self.endpoint
    }
}

/// Pipe instances restricted to the current user and SYSTEM.
///
/// Windows' default pipe DACL lets Everyone (and Anonymous) open the pipe for
/// reading, so every instance -- the first and each fresh one -- is created
/// with an explicit security descriptor. Same SDDL as the WhatsApp bridge's
/// `pipeSDDL` (Go side).
#[cfg(windows)]
mod pipe_security {
    use std::ffi::c_void;
    use std::io;
    use std::ptr::null_mut;

    use tokio::net::windows::named_pipe::{NamedPipeServer, ServerOptions};
    use windows_sys::Win32::Foundation::LocalFree;
    use windows_sys::Win32::Security::Authorization::{
        ConvertStringSecurityDescriptorToSecurityDescriptorW, SDDL_REVISION_1,
    };
    use windows_sys::Win32::Security::{PSECURITY_DESCRIPTOR, SECURITY_ATTRIBUTES};

    /// Protected DACL: generic-all for the owner and for SYSTEM, nobody else.
    const PIPE_SDDL: &str = "D:P(A;;GA;;;OW)(A;;GA;;;SY)";

    /// Create one server instance of `endpoint`, refusing remote clients.
    /// `first` also refuses a name some other process already owns.
    pub(crate) fn create_instance(endpoint: &str, first: bool) -> io::Result<NamedPipeServer> {
        let sddl: Vec<u16> = PIPE_SDDL.encode_utf16().chain(Some(0)).collect();
        let mut sd: PSECURITY_DESCRIPTOR = null_mut();
        // SAFETY: `sddl` is NUL-terminated and outlives the call; `sd` is a
        // valid out-pointer. On success the descriptor is LocalAlloc'ed and
        // freed below.
        let ok = unsafe {
            ConvertStringSecurityDescriptorToSecurityDescriptorW(
                sddl.as_ptr(),
                SDDL_REVISION_1,
                &mut sd,
                null_mut(),
            )
        };
        if ok == 0 {
            return Err(io::Error::last_os_error());
        }
        let mut attrs = SECURITY_ATTRIBUTES {
            nLength: std::mem::size_of::<SECURITY_ATTRIBUTES>() as u32,
            lpSecurityDescriptor: sd,
            bInheritHandle: 0,
        };
        let mut opts = ServerOptions::new();
        opts.reject_remote_clients(true);
        if first {
            opts.first_pipe_instance(true);
        }
        // SAFETY: `attrs` is a valid SECURITY_ATTRIBUTES whose descriptor
        // stays alive until after the call; CreateNamedPipeW copies it.
        let created = unsafe {
            opts.create_with_security_attributes_raw(
                endpoint,
                &mut attrs as *mut SECURITY_ATTRIBUTES as *mut c_void,
            )
        };
        // SAFETY: `sd` came from the conversion above and is freed once.
        unsafe { LocalFree(sd as _) };
        created
    }
}

/// A fresh endpoint for tests: a socket file inside `dir` on Unix, a pipe
/// name unique to this process and call on Windows.
pub fn unique_test_endpoint(dir: &std::path::Path, name: &str) -> String {
    #[cfg(unix)]
    {
        dir.join(format!("{name}.sock"))
            .to_string_lossy()
            .to_string()
    }
    #[cfg(windows)]
    {
        use std::sync::atomic::{AtomicUsize, Ordering};
        static N: AtomicUsize = AtomicUsize::new(0);
        let _ = dir;
        format!(
            "{PIPE_PREFIX}mtw-test-{name}-{}-{}",
            std::process::id(),
            N.fetch_add(1, Ordering::Relaxed)
        )
    }
}
