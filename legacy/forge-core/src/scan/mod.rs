pub mod rules;

use crate::scan::rules::RULES;
use ignore::WalkBuilder;
use serde::Serialize;
use sha2::{Sha256, Digest};
use std::path::{Path, PathBuf};

const SCANNABLE_EXTENSIONS: &[&str] = &[
    "py", "js", "ts", "tsx", "jsx", "go", "rs", "java",
    "yml", "yaml", "json", "toml", "ini", "cfg", "conf",
    "env", "sh", "bash", "tf", "tfvars",
];

const SCANNABLE_NAMES: &[&str] = &[".env", ".npmrc", ".pypirc"];

const MAX_FILE_SIZE: u64 = 1_000_000;

#[derive(Serialize)]
pub struct Finding {
    pub rule_id: String,
    pub provider: String,
    #[serde(rename = "type")]
    pub secret_type: String,
    pub file_path: String,
    pub line_number: usize,
    pub risk_level: String,
    pub description: String,
    pub fingerprint: String,
}

pub fn run(path: &str) {
    let root = Path::new(path);
    let files = walk_scannable(root);
    eprintln!("Scanning {} files...", files.len());

    let mut total = 0;
    for file_path in &files {
        if file_path.is_symlink() {
            continue;
        }
        let content = match std::fs::read_to_string(file_path) {
            Ok(c) => c,
            Err(_) => continue,
        };
        let rel_path = file_path.strip_prefix(root)
            .unwrap_or(file_path)
            .to_string_lossy()
            .to_string();

        for (line_num, line) in content.lines().enumerate() {
            for rule in RULES.iter() {
                if rule.regex.is_match(line) {
                    let finding = Finding {
                        rule_id: rule.id.to_string(),
                        provider: rule.provider.to_string(),
                        secret_type: rule.secret_type.to_string(),
                        file_path: rel_path.clone(),
                        line_number: line_num + 1,
                        risk_level: rule.risk_level.to_string(),
                        description: rule.description.to_string(),
                        fingerprint: fingerprint(line),
                    };
                    if let Ok(json) = serde_json::to_string(&finding) {
                        println!("{}", json);
                    }
                    total += 1;
                }
            }

            // High-entropy detection for generic secrets
            if has_high_entropy_secret(line) {
                let finding = Finding {
                    rule_id: "high-entropy".to_string(),
                    provider: "generic".to_string(),
                    secret_type: "secret".to_string(),
                    file_path: rel_path.clone(),
                    line_number: line_num + 1,
                    risk_level: "medium".to_string(),
                    description: "High-entropy string near secret keyword".to_string(),
                    fingerprint: fingerprint(line),
                };
                if let Ok(json) = serde_json::to_string(&finding) {
                    println!("{}", json);
                }
                total += 1;
            }
        }
    }
    eprintln!("Found {} potential secrets.", total);
}

fn walk_scannable(root: &Path) -> Vec<PathBuf> {
    let mut files = Vec::new();
    let walker = WalkBuilder::new(root)
        .hidden(false) // scan hidden files like .env
        .git_ignore(true)
        .build();
    for entry in walker.flatten() {
        let path = entry.path();
        if !path.is_file() || path.is_symlink() {
            continue;
        }
        if let Ok(meta) = path.metadata() {
            if meta.len() > MAX_FILE_SIZE {
                continue;
            }
        }
        let name = path.file_name().and_then(|n| n.to_str()).unwrap_or("");
        let ext = path.extension().and_then(|e| e.to_str()).unwrap_or("");
        if SCANNABLE_EXTENSIONS.contains(&ext) || SCANNABLE_NAMES.contains(&name) {
            files.push(path.to_path_buf());
        }
    }
    files
}

fn fingerprint(line: &str) -> String {
    let trimmed = line.trim();
    let mut hasher = Sha256::new();
    // Hash first 4 + last 4 chars (never store the actual secret)
    let safe = if trimmed.len() > 8 {
        format!("{}...{}", &trimmed[..4], &trimmed[trimmed.len()-4..])
    } else {
        trimmed.to_string()
    };
    hasher.update(&safe);
    hex::encode(hasher.finalize())
}

fn has_high_entropy_secret(line: &str) -> bool {
    let lower = line.to_lowercase();
    let has_keyword = lower.contains("password") || lower.contains("secret")
        || lower.contains("token") || lower.contains("api_key")
        || lower.contains("apikey");
    if !has_keyword {
        return false;
    }
    // Look for a high-entropy value after = or :
    for part in line.split(&['=', ':', '"', '\''][..]) {
        let trimmed = part.trim();
        if trimmed.len() >= 16 && shannon_entropy(trimmed) > 4.5 {
            return true;
        }
    }
    false
}

fn shannon_entropy(s: &str) -> f64 {
    let mut freq = [0u32; 256];
    for &b in s.as_bytes() {
        freq[b as usize] += 1;
    }
    let len = s.len() as f64;
    let mut entropy = 0.0;
    for &count in &freq {
        if count > 0 {
            let p = count as f64 / len;
            entropy -= p * p.log2();
        }
    }
    entropy
}

pub fn watch(path: &str, interval_secs: u64) {
    use std::collections::HashSet;
    use std::thread;
    use std::time::Duration;

    eprintln!("=== Security Monitor: watching {} ({}s interval) ===", path, interval_secs);
    let mut known: HashSet<String> = HashSet::new();

    loop {
        let root = std::path::Path::new(path);
        let files = walk_scannable(root);
        let mut new_count = 0;

        for file_path in &files {
            if file_path.is_symlink() { continue; }
            let content = match std::fs::read_to_string(file_path) {
                Ok(c) => c, Err(_) => continue,
            };
            let rel = file_path.strip_prefix(root).unwrap_or(file_path)
                .to_string_lossy().to_string();

            for (ln, line) in content.lines().enumerate() {
                for rule in RULES.iter() {
                    if rule.regex.is_match(line) {
                        let fp = fingerprint(line);
                        if known.insert(fp.clone()) {
                            let f = Finding {
                                rule_id: rule.id.to_string(),
                                provider: rule.provider.to_string(),
                                secret_type: rule.secret_type.to_string(),
                                file_path: rel.clone(),
                                line_number: ln + 1,
                                risk_level: rule.risk_level.to_string(),
                                description: rule.description.to_string(),
                                fingerprint: fp,
                            };
                            if let Ok(json) = serde_json::to_string(&f) { println!("{}", json); }
                            new_count += 1;
                        }
                    }
                }
            }
        }

        if new_count > 0 {
            let ts = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH).unwrap().as_secs();
            eprintln!("[{}] {} new secret(s)", ts, new_count);
        }

        thread::sleep(Duration::from_secs(interval_secs));
    }
}
