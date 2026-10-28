#![cfg(all(feature = "tachyon", unix))]

use std::{
    io::{Read, Write},
    net::TcpListener,
    path::Path,
    process::{Child, Command, Output, Stdio},
    thread,
    time::{Duration, Instant},
};

use serde_json::{Value, json};

struct Process(Option<Child>);

impl Process {
    fn wait(mut self) -> Output {
        let started = Instant::now();
        loop {
            if self.0.as_mut().unwrap().try_wait().unwrap().is_some() {
                return self.0.take().unwrap().wait_with_output().unwrap();
            }
            assert!(
                started.elapsed() < Duration::from_secs(5),
                "CLI did not exit promptly"
            );
            thread::sleep(Duration::from_millis(10));
        }
    }

    fn signal(&self, signal: &str) {
        assert!(
            Command::new("kill")
                .args([signal, &self.0.as_ref().unwrap().id().to_string()])
                .status()
                .unwrap()
                .success()
        );
    }
}

impl Drop for Process {
    fn drop(&mut self) {
        if let Some(mut child) = self.0.take() {
            let _ = child.kill();
            let _ = child.wait();
        }
    }
}

fn cli(dir: &Path, args: &[&str]) -> Process {
    Process(Some(
        Command::new(env!("CARGO_BIN_EXE_zcash-devtool"))
            .args(["tachyon", "--wallet-dir"])
            .arg(dir)
            .args(args)
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .unwrap(),
    ))
}

fn wallet(url: &str) -> tempfile::TempDir {
    use std::os::unix::fs::PermissionsExt;
    let dir = tempfile::tempdir().unwrap();
    std::fs::set_permissions(dir.path(), std::fs::Permissions::from_mode(0o700)).unwrap();
    let db = rusqlite::Connection::open(dir.path().join("tachyon.sqlite")).unwrap();
    db.execute_batch("CREATE TABLE config (id INTEGER PRIMARY KEY, data TEXT NOT NULL);")
        .unwrap();
    let config = json!({"version":1,"rpc_url":url,"cookie_file":null,
        "activation":8,"activation_hash":"11".repeat(32),"genesis":"00".repeat(32),
        "network_id":"test-only","seed":vec![7u8;32]});
    db.execute("INSERT INTO config VALUES(1, ?)", [config.to_string()])
        .unwrap();
    dir
}

#[test]
fn progress_is_stderr_only_and_quiet_preserves_the_json_result() {
    let dir = wallet("http://127.0.0.1:1");
    let normal = cli(dir.path(), &["address"]).wait();
    assert!(
        normal.status.success(),
        "{}",
        String::from_utf8_lossy(&normal.stderr)
    );
    let result: Value = serde_json::from_slice(&normal.stdout).unwrap();
    assert!(result["funding_address"].as_str().unwrap().starts_with('t'));
    assert!(String::from_utf8_lossy(&normal.stderr).contains("Opening wallet database"));
    for args in [&["address", "--quiet"][..], &["-q", "address"][..]] {
        let quiet = cli(dir.path(), args).wait();
        assert!(quiet.status.success());
        assert!(quiet.stderr.is_empty());
        assert_eq!(
            serde_json::from_slice::<Value>(&quiet.stdout).unwrap(),
            result
        );
    }
    let missing = dir.path().join("missing");
    let error = cli(&missing, &["address", "--quiet"]).wait();
    assert!(!error.status.success());
    assert!(error.stdout.is_empty());
    assert!(!error.stderr.is_empty(), "quiet must not hide errors");
}

#[test]
fn ctrl_c_and_sigterm_cancel_stalled_rpc_even_in_quiet_mode() {
    for (signal, code) in [("-INT", 130), ("-TERM", 143)] {
        for partial_body in [false, true] {
            let listener = TcpListener::bind("127.0.0.1:0").unwrap();
            listener.set_nonblocking(true).unwrap();
            let dir = wallet(&format!("http://{}", listener.local_addr().unwrap()));
            let process = cli(dir.path(), &["sync", "--quiet"]);
            let started = Instant::now();
            let mut connection = loop {
                match listener.accept() {
                    Ok((connection, _)) => break connection,
                    Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                        assert!(
                            started.elapsed() < Duration::from_secs(5),
                            "CLI never requested RPC"
                        );
                        thread::sleep(Duration::from_millis(10));
                    }
                    Err(e) => panic!("accept: {e}"),
                }
            };
            connection
                .set_read_timeout(Some(Duration::from_secs(5)))
                .unwrap();
            let mut request = [0; 4096];
            assert!(connection.read(&mut request).unwrap() > 0);
            if partial_body {
                connection
                    .write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 1000\r\n\r\n{")
                    .unwrap();
            }
            process.signal(signal);
            let output = process.wait();
            assert_eq!(
                output.status.code(),
                Some(code),
                "{}",
                String::from_utf8_lossy(&output.stderr)
            );
            assert!(output.stdout.is_empty());
            let stderr = String::from_utf8_lossy(&output.stderr);
            assert!(stderr.contains("Cancelling"));
            assert!(stderr.contains("uncommitted work rolled back"));
            assert!(!stderr.contains("Requesting"));
            // The writer exited and released the SQLite transaction/lock.
            assert!(cli(dir.path(), &["address", "-q"]).wait().status.success());
        }
    }
}
