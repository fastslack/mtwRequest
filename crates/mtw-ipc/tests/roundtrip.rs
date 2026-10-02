use mtw_ipc::{connect, is_pipe, unique_test_endpoint, IpcListener, PIPE_PREFIX};
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};

#[test]
fn pipe_prefix_decides_the_kind() {
    assert!(is_pipe(r"\\.\pipe\mtw-whatsapp"));
    assert!(!is_pipe("/var/run/mtw-whatsapp/whatsapp.sock"));
    assert!(!is_pipe(r"C:\Users\me\run\whatsapp.sock"));
    assert_eq!(PIPE_PREFIX, r"\\.\pipe\");
}

/// Echo server: answers every line with "echo:<line>".
async fn serve_echo(mut listener: IpcListener, clients: usize) {
    for _ in 0..clients {
        let stream = listener.accept().await.expect("accept");
        tokio::spawn(async move {
            let (r, mut w) = tokio::io::split(stream);
            let mut lines = BufReader::new(r).lines();
            while let Ok(Some(line)) = lines.next_line().await {
                w.write_all(format!("echo:{line}\n").as_bytes()).await.unwrap();
            }
        });
    }
}

async fn ask(endpoint: &str, msg: &str) -> String {
    let stream = connect(endpoint).await.expect("connect");
    let (r, mut w) = tokio::io::split(stream);
    w.write_all(format!("{msg}\n").as_bytes()).await.unwrap();
    let mut line = String::new();
    BufReader::new(r).read_line(&mut line).await.unwrap();
    line.trim_end().to_string()
}

#[tokio::test]
async fn round_trip_and_sequential_reconnects() {
    let dir = tempfile::tempdir().unwrap();
    let endpoint = unique_test_endpoint(dir.path(), "roundtrip");
    let listener = IpcListener::bind(&endpoint).expect("bind");
    assert_eq!(listener.endpoint(), endpoint);
    let server = tokio::spawn(serve_echo(listener, 3));

    // Three clients one after another: a reconnecting peer must always find
    // the endpoint accepting again (Review Focus 1).
    for i in 0..3 {
        assert_eq!(ask(&endpoint, &format!("ping{i}")).await, format!("echo:ping{i}"));
    }
    server.await.unwrap();
}

#[tokio::test]
async fn wrong_kind_for_this_platform_is_a_clear_error() {
    #[cfg(unix)]
    let wrong = r"\\.\pipe\mtw-wrong-kind";
    #[cfg(windows)]
    let wrong = "/tmp/mtw-wrong-kind.sock";

    let err = connect(wrong).await.err().expect("must fail");
    assert_eq!(err.kind(), std::io::ErrorKind::InvalidInput);
    let err = IpcListener::bind(wrong).err().expect("must fail");
    assert_eq!(err.kind(), std::io::ErrorKind::InvalidInput);
}

#[cfg(unix)]
#[tokio::test]
async fn bind_replaces_a_stale_socket_file_and_creates_the_dir() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("nested").join("stale.sock");
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(&path, b"stale").unwrap();
    let endpoint = path.to_string_lossy().to_string();
    let listener = IpcListener::bind(&endpoint).expect("bind over a stale file");
    tokio::spawn(serve_echo(listener, 1));
    assert_eq!(ask(&endpoint, "hi").await, "echo:hi");
}
