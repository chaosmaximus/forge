pub mod loop_runner;
use serde::Serialize;

#[derive(Serialize)]
pub struct ResearchResult {
    pub topic: String,
    pub branch: String,
    pub origin_ref: String,
    pub iterations: usize,
    pub findings: Vec<Finding>,
    pub conclusion: String,
}

#[derive(Serialize)]
pub struct Finding {
    pub iteration: usize,
    pub action: String,
    pub result: String,
    pub kept: bool,
    pub checkpoint: String,
}

pub fn run(topic: &str, max_iterations: usize, workdir: &str) {
    eprintln!("=== AutoResearch: {} ===", topic);
    eprintln!("Max iterations: {}, workdir: {}", max_iterations, workdir);
    let result = loop_runner::execute(topic, max_iterations, workdir);
    println!("{}", serde_json::to_string_pretty(&result).unwrap());
}

/// Discard the most recent research iteration (revert last commit).
pub fn discard(workdir: &str) {
    eprintln!("=== AutoResearch: discard last iteration ===");
    match loop_runner::discard_last(workdir) {
        Ok(msg) => {
            println!("{}", serde_json::json!({
                "status": "discarded",
                "message": msg
            }));
        }
        Err(e) => {
            eprintln!("Error discarding: {}", e);
            println!("{}", serde_json::json!({
                "status": "error",
                "message": e
            }));
        }
    }
}
