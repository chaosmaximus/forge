//! `forge test check` — page health check via curl (+ optional Playwright screenshot).

use std::process::Command;
use std::time::Instant;

pub fn run(url: &str, screenshot: Option<&str>, format: &str) {
    let start = Instant::now();

    // Use curl for HTTP status + headers + body
    let output = Command::new("curl")
        .args([
            "-s",               // silent
            "-o", "/dev/stdout", // body to stdout
            "-w", "\n__FORGE_CURL_META__\nhttp_code:%{http_code}\ntime_total:%{time_total}\nredirect_url:%{redirect_url}\ncontent_type:%{content_type}\n",
            "-L",               // follow redirects
            "--max-time", "30", // timeout
            url,
        ])
        .output();

    let duration = start.elapsed().as_millis() as u64;

    match output {
        Ok(out) => {
            let raw = String::from_utf8_lossy(&out.stdout).to_string();
            let stderr = String::from_utf8_lossy(&out.stderr).to_string();

            let result = parse_curl_output(&raw, &stderr, url, duration);

            // Optional screenshot
            let screenshot_result = if let Some(file) = screenshot {
                take_screenshot_for_check(url, file)
            } else {
                None
            };

            output_check_result(&result, screenshot_result.as_deref(), format);
        }
        Err(_) => {
            if format == "text" {
                eprintln!("Error: curl not found");
            } else {
                println!(
                    "{}",
                    serde_json::json!({
                        "error": "curl not found",
                        "url": url
                    })
                );
            }
        }
    }
}

#[derive(serde::Serialize)]
struct CheckResult {
    url: String,
    status: u16,
    response_ms: u64,
    content_type: String,
    errors: Vec<String>,
    warnings: Vec<String>,
}

fn parse_curl_output(raw: &str, stderr: &str, url: &str, duration_ms: u64) -> CheckResult {
    let mut status: u16 = 0;
    let mut response_ms = duration_ms;
    let mut content_type = String::new();
    let body;

    // Split on our meta marker
    if let Some(idx) = raw.find("__FORGE_CURL_META__") {
        body = raw[..idx].to_string();
        let meta = &raw[idx..];
        for line in meta.lines() {
            if let Some(code) = line.strip_prefix("http_code:") {
                status = code.trim().parse().unwrap_or(0);
            }
            if let Some(time) = line.strip_prefix("time_total:") {
                if let Ok(secs) = time.trim().parse::<f64>() {
                    response_ms = (secs * 1000.0) as u64;
                }
            }
            if let Some(ct) = line.strip_prefix("content_type:") {
                content_type = ct.trim().to_string();
            }
        }
    } else {
        body = raw.to_string();
    }

    let mut errors = Vec::new();
    let mut warnings = Vec::new();

    // Check HTTP status
    if status == 0 {
        errors.push(format!("Connection failed: {}", stderr.lines().next().unwrap_or("unknown error")));
    } else if status >= 500 {
        errors.push(format!("Server error: HTTP {}", status));
    } else if status >= 400 {
        errors.push(format!("Client error: HTTP {}", status));
    }

    // Check body for common error patterns
    let body_lower = body.to_lowercase();
    let error_patterns = [
        ("internal server error", "Page contains 'Internal Server Error'"),
        ("502 bad gateway", "Page contains '502 Bad Gateway'"),
        ("503 service unavailable", "Page contains '503 Service Unavailable'"),
        ("fatal error", "Page contains 'Fatal Error'"),
        ("stack trace", "Page contains stack trace"),
        ("traceback (most recent call last)", "Page contains Python traceback"),
        ("unhandled exception", "Page contains unhandled exception"),
    ];

    for (pattern, msg) in &error_patterns {
        if body_lower.contains(pattern) {
            errors.push(msg.to_string());
        }
    }

    let warning_patterns = [
        ("deprecated", "Page mentions 'deprecated'"),
        ("warning:", "Page contains warning message"),
        ("mixed content", "Page has mixed content warning"),
    ];

    for (pattern, msg) in &warning_patterns {
        if body_lower.contains(pattern) {
            warnings.push(msg.to_string());
        }
    }

    // Check response time
    if response_ms > 5000 {
        warnings.push(format!("Slow response: {}ms", response_ms));
    }

    // Check for empty body
    if body.trim().is_empty() && status < 300 {
        warnings.push("Empty response body".to_string());
    }

    CheckResult {
        url: url.to_string(),
        status,
        response_ms,
        content_type,
        errors,
        warnings,
    }
}

fn take_screenshot_for_check(url: &str, file: &str) -> Option<String> {
    // Check if npx/playwright is available
    let result = Command::new("npx")
        .args(["playwright", "screenshot", url, file, "--full-page"])
        .output();

    match result {
        Ok(out) if out.status.success() => Some(file.to_string()),
        _ => None,
    }
}

fn output_check_result(result: &CheckResult, screenshot: Option<&str>, format: &str) {
    if format == "text" {
        let status_label = match result.status {
            200..=299 => "OK",
            300..=399 => "REDIRECT",
            400..=499 => "CLIENT ERROR",
            500..=599 => "SERVER ERROR",
            _ => "UNKNOWN",
        };
        println!(
            "{} {} — HTTP {} [{}ms]",
            status_label, result.url, result.status, result.response_ms
        );
        if !result.content_type.is_empty() {
            println!("  Content-Type: {}", result.content_type);
        }
        for e in &result.errors {
            println!("  ERROR: {}", e);
        }
        for w in &result.warnings {
            println!("  WARN:  {}", w);
        }
        if let Some(s) = screenshot {
            println!("  Screenshot: {}", s);
        }
        if result.errors.is_empty() && result.warnings.is_empty() {
            println!("  No issues detected");
        }
    } else {
        let mut val = serde_json::to_value(result).unwrap_or(serde_json::json!({}));
        if let Some(s) = screenshot {
            val.as_object_mut()
                .unwrap()
                .insert("screenshot".to_string(), serde_json::json!(s));
        }
        println!("{}", val);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_curl_success() {
        let raw = "<html>Hello</html>\n__FORGE_CURL_META__\nhttp_code:200\ntime_total:0.150\nredirect_url:\ncontent_type:text/html; charset=utf-8\n";
        let result = parse_curl_output(raw, "", "https://example.com", 150);
        assert_eq!(result.status, 200);
        assert_eq!(result.response_ms, 150);
        assert!(result.errors.is_empty());
        assert_eq!(result.content_type, "text/html; charset=utf-8");
    }

    #[test]
    fn test_parse_curl_500() {
        let raw = "Internal Server Error\n__FORGE_CURL_META__\nhttp_code:500\ntime_total:0.050\nredirect_url:\ncontent_type:text/html\n";
        let result = parse_curl_output(raw, "", "https://example.com", 50);
        assert_eq!(result.status, 500);
        assert!(!result.errors.is_empty());
        // Should have both the HTTP 500 error AND the body pattern match
        assert!(result.errors.iter().any(|e| e.contains("Server error")));
        assert!(result.errors.iter().any(|e| e.contains("Internal Server Error")));
    }

    #[test]
    fn test_parse_curl_404() {
        let raw = "Not Found\n__FORGE_CURL_META__\nhttp_code:404\ntime_total:0.100\nredirect_url:\ncontent_type:text/html\n";
        let result = parse_curl_output(raw, "", "https://example.com/missing", 100);
        assert_eq!(result.status, 404);
        assert!(result.errors.iter().any(|e| e.contains("Client error")));
    }

    #[test]
    fn test_parse_curl_connection_failure() {
        let raw = "";
        let result = parse_curl_output(raw, "Could not resolve host", "https://bad.invalid", 5000);
        assert_eq!(result.status, 0);
        assert!(result.errors.iter().any(|e| e.contains("Connection failed")));
    }

    #[test]
    fn test_slow_response_warning() {
        let raw = "<html>OK</html>\n__FORGE_CURL_META__\nhttp_code:200\ntime_total:6.500\nredirect_url:\ncontent_type:text/html\n";
        let result = parse_curl_output(raw, "", "https://slow.example.com", 6500);
        assert!(result.warnings.iter().any(|w| w.contains("Slow response")));
    }

    #[test]
    fn test_traceback_detection() {
        let raw = "Traceback (most recent call last):\n  File ...\n__FORGE_CURL_META__\nhttp_code:500\ntime_total:0.100\nredirect_url:\ncontent_type:text/html\n";
        let result = parse_curl_output(raw, "", "https://example.com", 100);
        assert!(result.errors.iter().any(|e| e.contains("Python traceback")));
    }

    #[test]
    fn test_empty_body_warning() {
        let raw = "\n__FORGE_CURL_META__\nhttp_code:200\ntime_total:0.050\nredirect_url:\ncontent_type:text/html\n";
        let result = parse_curl_output(raw, "", "https://example.com", 50);
        assert!(result.warnings.iter().any(|w| w.contains("Empty response")));
    }
}
