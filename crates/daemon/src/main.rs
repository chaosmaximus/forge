use forge_daemon::server::{DaemonState, run_server};
use std::sync::Arc;
use tokio::sync::{watch, Mutex};

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

    // I2: Set directory permissions to 0700 (owner-only access)
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let _ = std::fs::set_permissions(&dir, std::fs::Permissions::from_mode(0o700));
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

    // C1: Create shutdown watch channel
    let (shutdown_tx, _shutdown_rx) = watch::channel(false);

    // I3: Spawn signal handler that sends on shutdown channel instead of process::exit
    let shutdown_for_signal = shutdown_tx.clone();
    tokio::spawn(async move {
        tokio::signal::ctrl_c().await.ok();
        eprintln!("[daemon] shutting down (signal)");
        let _ = shutdown_for_signal.send(true);
    });

    eprintln!("forge-daemon: pid={pid} socket={socket_path} db={db_path}");

    // Run the server (returns when shutdown signal received or IO error)
    if let Err(e) = run_server(&socket_path, state, shutdown_tx).await {
        eprintln!("error: server failed: {e}");
    }

    // Graceful cleanup after server stops
    let _ = std::fs::remove_file(&socket_path);
    let _ = std::fs::remove_file(&pid_path);
    eprintln!("[daemon] stopped");
}
