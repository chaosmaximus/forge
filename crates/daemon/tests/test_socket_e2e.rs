// test_socket_e2e.rs — True end-to-end tests via Unix domain socket
//
// These tests start an actual daemon process and communicate via the socket
// protocol, validating the full request/response cycle including serialization.

use std::io::{BufRead, BufReader, Write};
use std::os::unix::net::UnixStream;
use std::path::PathBuf;
use std::process::{Child, Command, Stdio};
use std::time::Duration;

/// Test harness: starts a daemon with a temp socket/db, provides send/recv helpers.
struct TestDaemon {
    process: Child,
    socket_path: PathBuf,
    _db_path: PathBuf,
    _pid_path: PathBuf,
    _dir: tempfile::TempDir,
}

impl TestDaemon {
    fn start() -> Self {
        let dir = tempfile::tempdir().expect("create temp dir");
        let socket_path = dir.path().join("test.sock");
        let db_path = dir.path().join("test.db");
        let pid_path = dir.path().join("test.pid");

        // Find the daemon binary
        let daemon_bin = find_daemon_binary();

        // Set HOME to temp dir so PID file, forge dir, etc. go there
        // This avoids conflicts with the production daemon
        let forge_dir = dir.path().join(".forge");
        std::fs::create_dir_all(&forge_dir).expect("create .forge dir");

        let mut process = Command::new(&daemon_bin)
            .env("HOME", dir.path().to_str().unwrap())
            .env("FORGE_SOCKET", socket_path.to_str().unwrap())
            .env("FORGE_DB", db_path.to_str().unwrap())
            .stdout(Stdio::null())
            .stderr(Stdio::piped())
            .spawn()
            .unwrap_or_else(|e| panic!("failed to start daemon at {daemon_bin}: {e}"));

        // Wait for socket to appear (up to 30 seconds — release daemons bind
        // in <1s, but cold debug builds can take ~10-15s due to SQLite migrate
        // + worker spawn + ORT dynamic link).
        for _ in 0..300 {
            if socket_path.exists() {
                // Small extra delay for the daemon to fully bind
                std::thread::sleep(Duration::from_millis(100));
                return TestDaemon {
                    process,
                    socket_path,
                    _db_path: db_path,
                    _pid_path: pid_path,
                    _dir: dir,
                };
            }
            std::thread::sleep(Duration::from_millis(100));
        }

        // Socket never appeared — reap the daemon before panicking so clippy's
        // zombie_processes lint is satisfied and no orphan daemon remains.
        let _ = process.kill();
        let _ = process.wait();
        panic!("daemon did not create socket within 30 seconds");
    }

    /// Send a JSON request and receive a JSON response.
    fn request(&self, json: &str) -> serde_json::Value {
        let mut stream = UnixStream::connect(&self.socket_path).expect("connect to daemon socket");
        stream
            .set_read_timeout(Some(Duration::from_secs(10)))
            .unwrap();
        stream
            .set_write_timeout(Some(Duration::from_secs(5)))
            .unwrap();

        // Send request (NDJSON: one line)
        writeln!(stream, "{json}").expect("write request");
        stream.flush().expect("flush");

        // Read response line
        let mut reader = BufReader::new(&stream);
        let mut response = String::new();
        reader.read_line(&mut response).expect("read response");

        serde_json::from_str(&response)
            .unwrap_or_else(|e| panic!("parse response JSON: {e}\nraw: {response}"))
    }

    fn shutdown(&mut self) {
        let resp = self.request(r#"{"method":"shutdown"}"#);
        assert_eq!(resp["status"], "ok");
        // Give the daemon a moment to clean up
        std::thread::sleep(Duration::from_millis(200));
        // Kill if still running
        let _ = self.process.kill();
        let _ = self.process.wait();
    }
}

impl Drop for TestDaemon {
    fn drop(&mut self) {
        let _ = self.process.kill();
        let _ = self.process.wait();
    }
}

fn find_daemon_binary() -> String {
    let candidates = [
        "target/release/forge-daemon",
        "target/debug/forge-daemon",
        "../target/release/forge-daemon",
        "../../target/release/forge-daemon",
        "../../target/debug/forge-daemon",
    ];
    for c in &candidates {
        if std::path::Path::new(c).exists() {
            return c.to_string();
        }
    }
    if let Ok(d) = std::env::var("CARGO_MANIFEST_DIR") {
        for sub in &[
            "../../target/release/forge-daemon",
            "../../target/debug/forge-daemon",
        ] {
            let p = PathBuf::from(&d).join(sub);
            if p.exists() {
                return p.to_string_lossy().to_string();
            }
        }
    }
    panic!("forge-daemon binary not found — run `cargo build -p forge-daemon` (debug) or `cargo build --release -p forge-daemon` first");
}

#[test]
fn test_socket_remember_and_recall() {
    let mut daemon = TestDaemon::start();

    // Remember
    let resp = daemon.request(
        r#"{"method":"remember","params":{"memory_type":"decision","title":"Use JWT for auth","content":"Always use JWT tokens","confidence":0.95,"tags":["auth","jwt"]}}"#,
    );
    assert_eq!(resp["status"], "ok");
    assert_eq!(resp["data"]["kind"], "stored");
    let id = resp["data"]["id"].as_str().unwrap().to_string();
    assert!(!id.is_empty());

    // Recall
    let resp = daemon.request(r#"{"method":"recall","params":{"query":"JWT authentication"}}"#);
    assert_eq!(resp["status"], "ok");
    assert_eq!(resp["data"]["kind"], "memories");
    let count = resp["data"]["count"].as_u64().unwrap();
    assert!(count >= 1, "should find at least 1 memory");

    let results = resp["data"]["results"].as_array().unwrap();
    assert!(results.iter().any(|r| r["title"] == "Use JWT for auth"));

    daemon.shutdown();
}

#[test]
fn test_socket_health_and_doctor() {
    let mut daemon = TestDaemon::start();

    // Store 2 memories
    daemon.request(
        r#"{"method":"remember","params":{"memory_type":"decision","title":"Decision 1","content":"Content 1"}}"#,
    );
    daemon.request(
        r#"{"method":"remember","params":{"memory_type":"lesson","title":"Lesson 1","content":"Content 2"}}"#,
    );

    // Health
    let resp = daemon.request(r#"{"method":"health"}"#);
    assert_eq!(resp["status"], "ok");
    assert_eq!(resp["data"]["decisions"], 1);
    assert_eq!(resp["data"]["lessons"], 1);

    // Doctor
    let resp = daemon.request(r#"{"method":"doctor"}"#);
    assert_eq!(resp["status"], "ok");
    assert_eq!(resp["data"]["daemon_up"], true);
    assert_eq!(resp["data"]["memory_count"], 2);

    // Status
    let resp = daemon.request(r#"{"method":"status"}"#);
    assert_eq!(resp["status"], "ok");
    assert!(resp["data"]["uptime_secs"].is_u64());

    daemon.shutdown();
}

#[test]
fn test_socket_guardrails_check() {
    let mut daemon = TestDaemon::start();

    // Check on empty DB → safe
    let resp = daemon.request(
        r#"{"method":"guardrails_check","params":{"file":"src/auth.rs","action":"edit"}}"#,
    );
    assert_eq!(resp["status"], "ok");
    assert_eq!(resp["data"]["safe"], true);

    daemon.shutdown();
}

#[test]
fn test_socket_blast_radius() {
    let mut daemon = TestDaemon::start();

    let resp = daemon.request(r#"{"method":"blast_radius","params":{"file":"src/auth.rs"}}"#);
    assert_eq!(resp["status"], "ok");
    assert_eq!(resp["data"]["kind"], "blast_radius");
    assert_eq!(resp["data"]["callers"], 0);
    assert!(resp["data"]["decisions"].as_array().unwrap().is_empty());

    daemon.shutdown();
}

#[test]
fn test_socket_export_import_roundtrip() {
    let mut daemon = TestDaemon::start();

    // Store memories
    daemon.request(
        r#"{"method":"remember","params":{"memory_type":"decision","title":"Export test","content":"Should survive roundtrip","project":"forge"}}"#,
    );

    // Export
    let resp = daemon.request(r#"{"method":"export","params":{}}"#);
    assert_eq!(resp["status"], "ok");
    let memories = resp["data"]["memories"].as_array().unwrap();
    assert_eq!(memories.len(), 1);

    // Health by project
    let resp = daemon.request(r#"{"method":"health_by_project"}"#);
    assert_eq!(resp["status"], "ok");
    assert!(
        resp["data"]["projects"]["forge"]["decisions"]
            .as_u64()
            .unwrap()
            >= 1
    );

    daemon.shutdown();
}

#[test]
fn test_socket_forget_and_verify() {
    let mut daemon = TestDaemon::start();

    // Remember
    let resp = daemon.request(
        r#"{"method":"remember","params":{"memory_type":"decision","title":"To be forgotten","content":"Temp"}}"#,
    );
    let id = resp["data"]["id"].as_str().unwrap().to_string();

    // Verify it exists
    let resp = daemon.request(r#"{"method":"recall","params":{"query":"forgotten"}}"#);
    assert_eq!(resp["data"]["count"], 1);

    // Forget
    let forget_json = format!(r#"{{"method":"forget","params":{{"id":"{id}"}}}}"#);
    let resp = daemon.request(&forget_json);
    assert_eq!(resp["status"], "ok");
    assert_eq!(resp["data"]["kind"], "forgotten");

    // Verify gone from recall
    let resp = daemon.request(r#"{"method":"recall","params":{"query":"forgotten"}}"#);
    assert_eq!(resp["data"]["count"], 0);

    daemon.shutdown();
}

#[test]
fn test_socket_malformed_request() {
    let mut daemon = TestDaemon::start();

    // Send garbage — daemon should not crash
    let stream = UnixStream::connect(&daemon.socket_path).unwrap();
    let mut writer = std::io::BufWriter::new(&stream);
    writeln!(writer, "this is not json").unwrap();
    writer.flush().unwrap();

    let mut reader = BufReader::new(&stream);
    let mut response = String::new();
    let _result = reader.read_line(&mut response);
    // Daemon may close the connection or send an error — either is fine
    // The important thing is it doesn't crash

    // Verify daemon is still alive by sending a valid request
    let resp = daemon.request(r#"{"method":"health"}"#);
    assert_eq!(resp["status"], "ok");

    daemon.shutdown();
}

#[test]
fn test_socket_concurrent_connections() {
    let mut daemon = TestDaemon::start();

    // Send 10 requests from separate connections concurrently
    let handles: Vec<_> = (0..10)
        .map(|i| {
            let socket_path = daemon.socket_path.clone();
            std::thread::spawn(move || {
                let mut stream = UnixStream::connect(&socket_path).expect("connect");
                stream.set_read_timeout(Some(Duration::from_secs(10))).unwrap();
                let req = format!(
                    r#"{{"method":"remember","params":{{"memory_type":"decision","title":"Concurrent {i}","content":"Test {i}"}}}}
"#
                );
                stream.write_all(req.as_bytes()).expect("write");
                stream.flush().expect("flush");

                let mut reader = BufReader::new(&stream);
                let mut resp = String::new();
                reader.read_line(&mut resp).expect("read");
                let json: serde_json::Value = serde_json::from_str(&resp).expect("parse");
                assert_eq!(json["status"], "ok");
            })
        })
        .collect();

    for h in handles {
        h.join().expect("thread join");
    }

    // Verify all 10 stored (dedup may merge some if titles collide — they won't here)
    let resp = daemon.request(r#"{"method":"health"}"#);
    assert_eq!(resp["data"]["decisions"], 10);

    daemon.shutdown();
}
