//! `forge test run` — run project tests with auto-detected framework.

use std::process::Command;
use std::time::Instant;

use super::detect::{self, TestFramework};

/// Normalized test result.
#[derive(serde::Serialize)]
struct TestResult {
    status: String,
    framework: String,
    total: u64,
    passed: u64,
    failed: u64,
    skipped: u64,
    duration_ms: u64,
    failures: Vec<TestFailure>,
}

#[derive(serde::Serialize)]
struct TestFailure {
    test: String,
    file: String,
    message: String,
}

pub fn run(path: &str, format: &str) {
    // Resolve to absolute path for reliable detection
    let abs_path = std::fs::canonicalize(path)
        .map(|p| p.to_string_lossy().to_string())
        .unwrap_or_else(|_| path.to_string());

    let framework = detect::detect_framework(&abs_path);
    match framework {
        Some(fw) => run_framework(fw, &abs_path, format),
        None => {
            let err = serde_json::json!({
                "error": "No test framework detected",
                "hint": "Supported: pytest, vitest, jest, cargo test, go test"
            });
            if format == "text" {
                eprintln!("Error: No test framework detected in {}", path);
                eprintln!("Supported: pytest, vitest, jest, cargo test, go test");
            } else {
                println!("{}", err);
            }
        }
    }
}

fn run_framework(fw: TestFramework, path: &str, format: &str) {
    match fw {
        TestFramework::Pytest => run_pytest(path, format),
        TestFramework::Vitest => run_vitest(path, format),
        TestFramework::Jest => run_jest(path, format),
        TestFramework::CargoTest => run_cargo_test(path, format),
        TestFramework::GoTest => run_go_test(path, format),
    }
}

// ─── pytest ─────────────────────────────────────────────────────────────────

fn run_pytest(path: &str, format: &str) {
    let start = Instant::now();
    let output = Command::new("python3")
        .args(["-m", "pytest", "--tb=short", "-q", path])
        .current_dir(path)
        .output();

    let duration = start.elapsed().as_millis() as u64;

    match output {
        Ok(out) => {
            let stdout = String::from_utf8_lossy(&out.stdout).to_string();
            let stderr = String::from_utf8_lossy(&out.stderr).to_string();
            let combined = format!("{}\n{}", stdout, stderr);
            let result = parse_pytest_output(&combined, duration);
            output_result(&result, format);
        }
        Err(_) => output_not_found("pytest", TestFramework::Pytest.install_hint(), format),
    }
}

fn parse_pytest_output(output: &str, duration_ms: u64) -> TestResult {
    let mut passed: u64 = 0;
    let mut failed: u64 = 0;
    let mut skipped: u64 = 0;
    let mut failures = Vec::new();

    // Parse the pytest summary line which ends with "in X.XXs" or similar.
    // Examples:
    //   "115 passed, 3 skipped in 10.85s"
    //   "3 passed, 2 failed in 2.50s"
    //   "10 errors in 0.18s"
    let re_passed = regex::Regex::new(r"(\d+) passed").unwrap();
    let re_failed = regex::Regex::new(r"(\d+) failed").unwrap();
    let re_skipped = regex::Regex::new(r"(\d+) skipped").unwrap();
    let re_error = regex::Regex::new(r"(\d+) error").unwrap();
    let re_summary = regex::Regex::new(r"in \d+[\.\d]*s").unwrap();

    // Only parse counts from the final summary line (contains "in X.XXs")
    for line in output.lines().rev() {
        if re_summary.is_match(line) {
            if let Some(cap) = re_passed.captures(line) {
                passed = cap[1].parse().unwrap_or(0);
            }
            if let Some(cap) = re_failed.captures(line) {
                failed = cap[1].parse().unwrap_or(0);
            }
            if let Some(cap) = re_skipped.captures(line) {
                skipped = cap[1].parse().unwrap_or(0);
            }
            if let Some(cap) = re_error.captures(line) {
                failed += cap[1].parse::<u64>().unwrap_or(0);
            }
            break;
        }
    }

    // Parse individual failure blocks: "FAILED tests/test_foo.py::test_bar - AssertionError..."
    let re_failure = regex::Regex::new(r"FAILED\s+([^\s:]+)::(\S+)\s*[-–]\s*(.*)").unwrap();
    for cap in re_failure.captures_iter(output) {
        failures.push(TestFailure {
            file: cap[1].to_string(),
            test: cap[2].to_string(),
            message: cap[3].to_string(),
        });
    }

    let total = passed + failed + skipped;
    let status = if failed > 0 { "fail" } else { "pass" };

    TestResult {
        status: status.to_string(),
        framework: "pytest".to_string(),
        total,
        passed,
        failed,
        skipped,
        duration_ms,
        failures,
    }
}

// ─── vitest ─────────────────────────────────────────────────────────────────

fn run_vitest(path: &str, format: &str) {
    let start = Instant::now();
    let output = Command::new("npx")
        .args(["vitest", "run", "--reporter=json", path])
        .current_dir(path)
        .output();

    let duration = start.elapsed().as_millis() as u64;

    match output {
        Ok(out) => {
            let stdout = String::from_utf8_lossy(&out.stdout).to_string();
            let result = parse_vitest_json(&stdout, duration);
            output_result(&result, format);
        }
        Err(_) => output_not_found("vitest", TestFramework::Vitest.install_hint(), format),
    }
}

fn parse_vitest_json(stdout: &str, duration_ms: u64) -> TestResult {
    // Vitest JSON reporter outputs a JSON object; find the first '{' to skip any preamble
    let json_start = stdout.find('{');
    if let Some(start) = json_start {
        let json_str = &stdout[start..];
        if let Ok(val) = serde_json::from_str::<serde_json::Value>(json_str) {
            let num_passed = val.get("numPassedTests")
                .and_then(|v| v.as_u64())
                .unwrap_or(0);
            let num_failed = val.get("numFailedTests")
                .and_then(|v| v.as_u64())
                .unwrap_or(0);
            let num_total = val.get("numTotalTests")
                .and_then(|v| v.as_u64())
                .unwrap_or(num_passed + num_failed);
            let skipped = num_total.saturating_sub(num_passed + num_failed);

            let mut failures = Vec::new();
            if let Some(suites) = val.get("testResults").and_then(|v| v.as_array()) {
                for suite in suites {
                    let file = suite.get("name")
                        .and_then(|v| v.as_str())
                        .unwrap_or("")
                        .to_string();
                    if let Some(results) = suite.get("assertionResults").and_then(|v| v.as_array()) {
                        for result in results {
                            let status = result.get("status").and_then(|v| v.as_str()).unwrap_or("");
                            if status == "failed" {
                                let test_name = result.get("fullName")
                                    .or_else(|| result.get("title"))
                                    .and_then(|v| v.as_str())
                                    .unwrap_or("")
                                    .to_string();
                                let message = result.get("failureMessages")
                                    .and_then(|v| v.as_array())
                                    .and_then(|arr| arr.first())
                                    .and_then(|v| v.as_str())
                                    .unwrap_or("")
                                    .lines()
                                    .next()
                                    .unwrap_or("")
                                    .to_string();
                                failures.push(TestFailure {
                                    test: test_name,
                                    file: file.clone(),
                                    message,
                                });
                            }
                        }
                    }
                }
            }

            let status = if num_failed > 0 { "fail" } else { "pass" };
            return TestResult {
                status: status.to_string(),
                framework: "vitest".to_string(),
                total: num_total,
                passed: num_passed,
                failed: num_failed,
                skipped,
                duration_ms,
                failures,
            };
        }
    }

    // Fallback: couldn't parse JSON
    TestResult {
        status: "fail".to_string(),
        framework: "vitest".to_string(),
        total: 0,
        passed: 0,
        failed: 0,
        skipped: 0,
        duration_ms,
        failures: vec![TestFailure {
            test: "".to_string(),
            file: "".to_string(),
            message: "Failed to parse vitest JSON output".to_string(),
        }],
    }
}

// ─── jest ───────────────────────────────────────────────────────────────────

fn run_jest(path: &str, format: &str) {
    let start = Instant::now();
    let output = Command::new("npx")
        .args(["jest", "--json", path])
        .current_dir(path)
        .output();

    let duration = start.elapsed().as_millis() as u64;

    match output {
        Ok(out) => {
            let stdout = String::from_utf8_lossy(&out.stdout).to_string();
            let result = parse_jest_json(&stdout, duration);
            output_result(&result, format);
        }
        Err(_) => output_not_found("jest", TestFramework::Jest.install_hint(), format),
    }
}

fn parse_jest_json(stdout: &str, duration_ms: u64) -> TestResult {
    // Jest --json outputs JSON; skip any preamble
    let json_start = stdout.find('{');
    if let Some(start) = json_start {
        let json_str = &stdout[start..];
        if let Ok(val) = serde_json::from_str::<serde_json::Value>(json_str) {
            let num_passed = val.get("numPassedTests")
                .and_then(|v| v.as_u64())
                .unwrap_or(0);
            let num_failed = val.get("numFailedTests")
                .and_then(|v| v.as_u64())
                .unwrap_or(0);
            let num_total = val.get("numTotalTests")
                .and_then(|v| v.as_u64())
                .unwrap_or(num_passed + num_failed);
            let num_pending = val.get("numPendingTests")
                .and_then(|v| v.as_u64())
                .unwrap_or(0);

            let mut failures = Vec::new();
            if let Some(suites) = val.get("testResults").and_then(|v| v.as_array()) {
                for suite in suites {
                    let file = suite.get("name")
                        .and_then(|v| v.as_str())
                        .unwrap_or("")
                        .to_string();
                    if let Some(results) = suite.get("assertionResults").and_then(|v| v.as_array()) {
                        for result in results {
                            let status = result.get("status").and_then(|v| v.as_str()).unwrap_or("");
                            if status == "failed" {
                                let test_name = result.get("fullName")
                                    .or_else(|| result.get("title"))
                                    .and_then(|v| v.as_str())
                                    .unwrap_or("")
                                    .to_string();
                                let message = result.get("failureMessages")
                                    .and_then(|v| v.as_array())
                                    .and_then(|arr| arr.first())
                                    .and_then(|v| v.as_str())
                                    .unwrap_or("")
                                    .lines()
                                    .next()
                                    .unwrap_or("")
                                    .to_string();
                                failures.push(TestFailure {
                                    test: test_name,
                                    file: file.clone(),
                                    message,
                                });
                            }
                        }
                    }
                }
            }

            let status = if num_failed > 0 { "fail" } else { "pass" };
            return TestResult {
                status: status.to_string(),
                framework: "jest".to_string(),
                total: num_total,
                passed: num_passed,
                failed: num_failed,
                skipped: num_pending,
                duration_ms,
                failures,
            };
        }
    }

    TestResult {
        status: "fail".to_string(),
        framework: "jest".to_string(),
        total: 0,
        passed: 0,
        failed: 0,
        skipped: 0,
        duration_ms,
        failures: vec![TestFailure {
            test: "".to_string(),
            file: "".to_string(),
            message: "Failed to parse jest JSON output".to_string(),
        }],
    }
}

// ─── cargo test ─────────────────────────────────────────────────────────────

fn run_cargo_test(path: &str, format: &str) {
    let start = Instant::now();
    // cargo test with -- --format json is unstable; use normal output and parse
    let output = Command::new("cargo")
        .args(["test"])
        .current_dir(path)
        .output();

    let duration = start.elapsed().as_millis() as u64;

    match output {
        Ok(out) => {
            let stdout = String::from_utf8_lossy(&out.stdout).to_string();
            let stderr = String::from_utf8_lossy(&out.stderr).to_string();
            let combined = format!("{}\n{}", stdout, stderr);
            let result = parse_cargo_test_output(&combined, duration);
            output_result(&result, format);
        }
        Err(_) => output_not_found("cargo", TestFramework::CargoTest.install_hint(), format),
    }
}

fn parse_cargo_test_output(output: &str, duration_ms: u64) -> TestResult {
    let mut passed: u64 = 0;
    let mut failed: u64 = 0;
    let mut skipped: u64 = 0;
    let mut failures = Vec::new();

    // Parse "test result: ok. X passed; Y failed; Z ignored"
    let re_result = regex::Regex::new(
        r"test result:.*?(\d+) passed;\s*(\d+) failed;\s*(\d+) ignored"
    ).unwrap();

    for cap in re_result.captures_iter(output) {
        passed += cap[1].parse::<u64>().unwrap_or(0);
        failed += cap[2].parse::<u64>().unwrap_or(0);
        skipped += cap[3].parse::<u64>().unwrap_or(0);
    }

    // Parse individual failures: "---- test_name stdout ----" followed by assertion message
    let re_fail_header = regex::Regex::new(r"---- (\S+) stdout ----").unwrap();
    let mut in_failure: Option<String> = None;
    let mut failure_lines: Vec<String> = Vec::new();

    for line in output.lines() {
        if let Some(cap) = re_fail_header.captures(line) {
            // Flush previous failure
            if let Some(test_name) = in_failure.take() {
                let msg = failure_lines.join("\n");
                failures.push(TestFailure {
                    test: test_name,
                    file: String::new(),
                    message: msg.trim().to_string(),
                });
                failure_lines.clear();
            }
            in_failure = Some(cap[1].to_string());
        } else if in_failure.is_some() {
            if line.starts_with("---- ") || line.starts_with("failures:") || line.starts_with("test result:") {
                if let Some(test_name) = in_failure.take() {
                    let msg = failure_lines.join("\n");
                    failures.push(TestFailure {
                        test: test_name,
                        file: String::new(),
                        message: msg.trim().to_string(),
                    });
                    failure_lines.clear();
                }
            } else {
                failure_lines.push(line.to_string());
            }
        }
    }
    // Flush last
    if let Some(test_name) = in_failure.take() {
        let msg = failure_lines.join("\n");
        failures.push(TestFailure {
            test: test_name,
            file: String::new(),
            message: msg.trim().to_string(),
        });
    }

    let total = passed + failed + skipped;
    let status = if failed > 0 { "fail" } else { "pass" };

    TestResult {
        status: status.to_string(),
        framework: "cargo_test".to_string(),
        total,
        passed,
        failed,
        skipped,
        duration_ms,
        failures,
    }
}

// ─── go test ────────────────────────────────────────────────────────────────

fn run_go_test(path: &str, format: &str) {
    let start = Instant::now();
    let output = Command::new("go")
        .args(["test", "-json", "./..."])
        .current_dir(path)
        .output();

    let duration = start.elapsed().as_millis() as u64;

    match output {
        Ok(out) => {
            let stdout = String::from_utf8_lossy(&out.stdout).to_string();
            let result = parse_go_test_json(&stdout, duration);
            output_result(&result, format);
        }
        Err(_) => output_not_found("go", TestFramework::GoTest.install_hint(), format),
    }
}

fn parse_go_test_json(stdout: &str, duration_ms: u64) -> TestResult {
    let mut passed: u64 = 0;
    let mut failed: u64 = 0;
    let mut skipped: u64 = 0;
    let mut failures = Vec::new();

    for line in stdout.lines() {
        if line.trim().is_empty() {
            continue;
        }
        if let Ok(val) = serde_json::from_str::<serde_json::Value>(line) {
            let action = val.get("Action").and_then(|v| v.as_str()).unwrap_or("");
            let test = val.get("Test").and_then(|v| v.as_str()).unwrap_or("");
            let pkg = val.get("Package").and_then(|v| v.as_str()).unwrap_or("");

            // Only count individual test results (not package-level)
            if !test.is_empty() {
                match action {
                    "pass" => passed += 1,
                    "fail" => {
                        failed += 1;
                        let output_text = val.get("Output")
                            .and_then(|v| v.as_str())
                            .unwrap_or("")
                            .to_string();
                        failures.push(TestFailure {
                            test: test.to_string(),
                            file: pkg.to_string(),
                            message: output_text.trim().to_string(),
                        });
                    }
                    "skip" => skipped += 1,
                    _ => {}
                }
            }
        }
    }

    let total = passed + failed + skipped;
    let status = if failed > 0 { "fail" } else { "pass" };

    TestResult {
        status: status.to_string(),
        framework: "go_test".to_string(),
        total,
        passed,
        failed,
        skipped,
        duration_ms,
        failures,
    }
}

// ─── output helpers ─────────────────────────────────────────────────────────

fn output_result(result: &TestResult, format: &str) {
    if format == "text" {
        let icon = if result.status == "pass" { "PASS" } else { "FAIL" };
        println!(
            "{} ({}) — {} total, {} passed, {} failed, {} skipped [{:.1}s]",
            icon,
            result.framework,
            result.total,
            result.passed,
            result.failed,
            result.skipped,
            result.duration_ms as f64 / 1000.0
        );
        for f in &result.failures {
            let location = if f.file.is_empty() {
                f.test.clone()
            } else {
                format!("{}::{}", f.file, f.test)
            };
            println!("  FAIL {}", location);
            // Show first line of message
            if let Some(first_line) = f.message.lines().next() {
                if !first_line.is_empty() {
                    println!("       {}", first_line);
                }
            }
        }
    } else {
        println!("{}", serde_json::to_string(result).unwrap_or_else(|_| "{}".to_string()));
    }
}

fn output_not_found(tool: &str, install_hint: &str, format: &str) {
    if format == "text" {
        eprintln!("Error: {} not found", tool);
        eprintln!("Install: {}", install_hint);
    } else {
        println!(
            "{}",
            serde_json::json!({
                "error": format!("{} not found", tool),
                "install": install_hint
            })
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_pytest_pass() {
        let output = r#"
tests/test_foo.py ..
tests/test_bar.py ...
5 passed in 1.23s
"#;
        let result = parse_pytest_output(output, 1230);
        assert_eq!(result.status, "pass");
        assert_eq!(result.passed, 5);
        assert_eq!(result.failed, 0);
        assert_eq!(result.total, 5);
        assert_eq!(result.framework, "pytest");
    }

    #[test]
    fn test_parse_pytest_fail() {
        let output = r#"
FAILED tests/test_foo.py::test_bar - AssertionError: expected 1, got 2
FAILED tests/test_baz.py::test_qux - ValueError: invalid
3 passed, 2 failed in 2.50s
"#;
        let result = parse_pytest_output(output, 2500);
        assert_eq!(result.status, "fail");
        assert_eq!(result.passed, 3);
        assert_eq!(result.failed, 2);
        assert_eq!(result.total, 5);
        assert_eq!(result.failures.len(), 2);
        assert_eq!(result.failures[0].test, "test_bar");
        assert_eq!(result.failures[0].file, "tests/test_foo.py");
    }

    #[test]
    fn test_parse_pytest_with_skipped() {
        let output = "10 passed, 1 failed, 3 skipped in 5.00s";
        let result = parse_pytest_output(output, 5000);
        assert_eq!(result.passed, 10);
        assert_eq!(result.failed, 1);
        assert_eq!(result.skipped, 3);
        assert_eq!(result.total, 14);
    }

    #[test]
    fn test_parse_pytest_errors() {
        let output = "10 passed, 2 error in 3.00s";
        let result = parse_pytest_output(output, 3000);
        assert_eq!(result.passed, 10);
        assert_eq!(result.failed, 2);
    }

    #[test]
    fn test_parse_cargo_test_pass() {
        let output = r#"
running 5 tests
test test_one ... ok
test test_two ... ok
test test_three ... ok
test test_four ... ok
test test_five ... ok

test result: ok. 5 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.01s
"#;
        let result = parse_cargo_test_output(output, 10);
        assert_eq!(result.status, "pass");
        assert_eq!(result.passed, 5);
        assert_eq!(result.failed, 0);
        assert_eq!(result.framework, "cargo_test");
    }

    #[test]
    fn test_parse_cargo_test_fail() {
        let output = r#"
running 3 tests
test test_one ... ok
test test_two ... FAILED
test test_three ... ok

failures:

---- test_two stdout ----
thread 'test_two' panicked at 'assertion failed: false'

failures:
    test_two

test result: FAILED. 2 passed; 1 failed; 0 ignored; 0 measured; 0 filtered out
"#;
        let result = parse_cargo_test_output(output, 50);
        assert_eq!(result.status, "fail");
        assert_eq!(result.passed, 2);
        assert_eq!(result.failed, 1);
        assert_eq!(result.failures.len(), 1);
        assert_eq!(result.failures[0].test, "test_two");
    }

    #[test]
    fn test_parse_go_test_json() {
        let output = r#"
{"Time":"2024-01-01T00:00:00Z","Action":"run","Package":"example.com/foo","Test":"TestFoo"}
{"Time":"2024-01-01T00:00:01Z","Action":"pass","Package":"example.com/foo","Test":"TestFoo","Elapsed":0.5}
{"Time":"2024-01-01T00:00:01Z","Action":"run","Package":"example.com/foo","Test":"TestBar"}
{"Time":"2024-01-01T00:00:02Z","Action":"fail","Package":"example.com/foo","Test":"TestBar","Elapsed":0.3}
{"Time":"2024-01-01T00:00:02Z","Action":"pass","Package":"example.com/foo","Elapsed":1.0}
"#;
        let result = parse_go_test_json(output, 1000);
        assert_eq!(result.status, "fail");
        assert_eq!(result.passed, 1);
        assert_eq!(result.failed, 1);
        assert_eq!(result.total, 2);
        assert_eq!(result.framework, "go_test");
    }

    #[test]
    fn test_parse_vitest_json() {
        let json = r#"{"numPassedTests":8,"numFailedTests":1,"numTotalTests":9,"testResults":[{"name":"src/foo.test.ts","assertionResults":[{"status":"failed","fullName":"foo > should work","failureMessages":["Expected true to be false\n  at ..."]}]}]}"#;
        let result = parse_vitest_json(json, 3000);
        assert_eq!(result.status, "fail");
        assert_eq!(result.passed, 8);
        assert_eq!(result.failed, 1);
        assert_eq!(result.total, 9);
        assert_eq!(result.failures.len(), 1);
        assert_eq!(result.failures[0].test, "foo > should work");
    }

    #[test]
    fn test_parse_jest_json() {
        let json = r#"{"numPassedTests":5,"numFailedTests":0,"numTotalTests":5,"numPendingTests":2,"testResults":[]}"#;
        let result = parse_jest_json(json, 2000);
        assert_eq!(result.status, "pass");
        assert_eq!(result.passed, 5);
        assert_eq!(result.failed, 0);
        assert_eq!(result.skipped, 2);
        assert_eq!(result.framework, "jest");
    }
}
