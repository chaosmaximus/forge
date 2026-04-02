pub mod loop_runner;
use serde::Serialize;

#[derive(Serialize)]
pub struct ResearchResult {
    pub topic: String,
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
}

pub fn run(topic: &str, max_iterations: usize, workdir: &str) {
    eprintln!("=== AutoResearch: {} ===", topic);
    eprintln!("Max iterations: {}, workdir: {}", max_iterations, workdir);
    let result = loop_runner::execute(topic, max_iterations, workdir);
    println!("{}", serde_json::to_string_pretty(&result).unwrap());
}
