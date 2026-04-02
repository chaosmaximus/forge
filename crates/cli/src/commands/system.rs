use crate::client;
use forge_v2_core::protocol::{Request, Response, ResponseData};

/// Print system health (memory counts by type + edges).
pub async fn health() {
    let request = Request::Health;

    match client::send(&request).await {
        Ok(Response::Ok {
            data:
                ResponseData::Health {
                    decisions,
                    lessons,
                    patterns,
                    preferences,
                    edges,
                },
        }) => {
            let total = decisions + lessons + patterns + preferences;
            println!("Health:");
            println!("  decisions:   {decisions}");
            println!("  lessons:     {lessons}");
            println!("  patterns:    {patterns}");
            println!("  preferences: {preferences}");
            println!("  total:       {total}");
            println!("  edges:       {edges}");
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
