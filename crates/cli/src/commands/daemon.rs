use crate::client;
use forge_v2_core::protocol::{Request, Response, ResponseData};

/// Print daemon status (uptime, workers, memory count).
pub async fn status() {
    let request = Request::Status;

    match client::send(&request).await {
        Ok(Response::Ok {
            data:
                ResponseData::Status {
                    uptime_secs,
                    workers,
                    memory_count,
                },
        }) => {
            let hours = uptime_secs / 3600;
            let minutes = (uptime_secs % 3600) / 60;
            let secs = uptime_secs % 60;
            println!("Daemon status:");
            println!("  uptime:   {hours}h {minutes}m {secs}s");
            println!("  memories: {memory_count}");
            if workers.is_empty() {
                println!("  workers:  (none)");
            } else {
                println!("  workers:  {}", workers.join(", "));
            }
        }
        Ok(Response::Error { message }) => {
            eprintln!("error: {message}");
            std::process::exit(1);
        }
        Ok(other) => {
            eprintln!("unexpected response: {other:?}");
            std::process::exit(1);
        }
        Err(e) => {
            eprintln!("error: {e}");
            std::process::exit(1);
        }
    }
}

/// Send shutdown signal to the daemon.
pub async fn stop() {
    let request = Request::Shutdown;

    match client::send(&request).await {
        Ok(Response::Ok {
            data: ResponseData::Shutdown,
        }) => {
            println!("Daemon shutdown requested.");
        }
        Ok(Response::Error { message }) => {
            eprintln!("error: {message}");
            std::process::exit(1);
        }
        Ok(other) => {
            eprintln!("unexpected response: {other:?}");
            std::process::exit(1);
        }
        Err(e) => {
            eprintln!("error: {e}");
            std::process::exit(1);
        }
    }
}
