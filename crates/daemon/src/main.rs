mod db;
mod graph;
mod recall;
mod server;
mod vector;

use server::{DaemonState, run_server};
use std::sync::Arc;
use tokio::sync::Mutex;

fn default_socket_path() -> String {
    let home = std::env::var("HOME").unwrap_or_else(|_| "/tmp".to_string());
    format!("{home}/.forge/forge.sock")
}

fn default_db_path() -> String {
    let home = std::env::var("HOME").unwrap_or_else(|_| "/tmp".to_string());
    format!("{home}/.forge/forge.db")
}

fn forge_dir() -> String {
    let home = std::env::var("HOME").unwrap_or_else(|_| "/tmp".to_string());
    format!("{home}/.forge")
}

fn pid_file_path() -> String {
    format!("{}/forge.pid", forge_dir())
}

#[tokio::main]
async fn main() {
    let socket_path = std::env::var("FORGE_SOCKET").unwrap_or_else(|_| default_socket_path());
    let db_path = std::env::var("FORGE_DB").unwrap_or_else(|_| default_db_path());

    // Ensure ~/.forge/ directory exists
    let dir = forge_dir();
    if let Err(e) = std::fs::create_dir_all(&dir) {
        eprintln!("error: failed to create {dir}: {e}");
        std::process::exit(1);
    }

    // Write PID file
    let pid = std::process::id();
    let pid_path = pid_file_path();
    if let Err(e) = std::fs::write(&pid_path, pid.to_string()) {
        eprintln!("error: failed to write PID file {pid_path}: {e}");
        std::process::exit(1);
    }

    // Create DaemonState (opens/creates DB)
    let state = match DaemonState::new(&db_path) {
        Ok(s) => s,
        Err(e) => {
            eprintln!("error: failed to open database {db_path}: {e}");
            let _ = std::fs::remove_file(&pid_path);
            std::process::exit(1);
        }
    };

    let state = Arc::new(Mutex::new(state));

    // Capture paths for signal handler cleanup
    let socket_cleanup = socket_path.clone();
    let pid_cleanup = pid_path.clone();

    // Spawn signal handler for graceful shutdown
    tokio::spawn(async move {
        if let Err(e) = tokio::signal::ctrl_c().await {
            eprintln!("error: signal handler failed: {e}");
        }
        eprintln!("\nforge-daemon: shutting down...");
        let _ = std::fs::remove_file(&socket_cleanup);
        let _ = std::fs::remove_file(&pid_cleanup);
        std::process::exit(0);
    });

    eprintln!("forge-daemon: pid={pid} socket={socket_path} db={db_path}");

    // Run the server (blocks forever unless IO error)
    if let Err(e) = run_server(&socket_path, state).await {
        eprintln!("error: server failed: {e}");
        let _ = std::fs::remove_file(&socket_path);
        let _ = std::fs::remove_file(&pid_path);
        std::process::exit(1);
    }
}
