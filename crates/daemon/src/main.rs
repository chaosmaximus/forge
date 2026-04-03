use forge_daemon::server::{DaemonState, run_server};
use forge_v2_core::{forge_dir, default_socket_path, default_db_path, default_pid_path};
use fs2::FileExt;
use std::io::Write;
use std::sync::Arc;
use tokio::sync::{watch, Mutex};

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

    // C2: Write PID file with advisory lock to prevent multiple daemon instances
    let pid_path = default_pid_path();
    let pid_file = match std::fs::OpenOptions::new()
        .create(true)
        .write(true)
        .truncate(true)
        .open(&pid_path)
    {
        Ok(f) => f,
        Err(e) => {
            eprintln!("error: failed to open PID file {pid_path}: {e}");
            std::process::exit(1);
        }
    };

    if pid_file.try_lock_exclusive().is_err() {
        eprintln!("error: another forge-daemon is already running (PID file locked)");
        std::process::exit(1);
    }

    // Write PID — file is now locked exclusively
    let pid = std::process::id();
    if let Err(e) = write!(&pid_file, "{}", pid) {
        eprintln!("error: failed to write PID to {pid_path}: {e}");
        std::process::exit(1);
    }

    // Keep pid_file alive (holds the advisory lock) for the lifetime of main()
    let _pid_file_guard = pid_file;

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

    // M6: Graceful cleanup after server stops (both success and error paths)
    let _ = std::fs::remove_file(&socket_path);
    let _ = std::fs::remove_file(&pid_path);
    eprintln!("[daemon] stopped");
    // _pid_file_guard drops here, releasing the advisory lock
}
