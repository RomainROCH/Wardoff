#![cfg(windows)]

use std::fs;
use std::path::PathBuf;
use std::process::{Command, Output};
use std::time::{SystemTime, UNIX_EPOCH};

struct TempFixture {
    root: PathBuf,
}

impl TempFixture {
    fn new(name: &str) -> Self {
        let unique = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("system clock should be after the Unix epoch")
            .as_nanos();
        let root = std::env::temp_dir().join(format!("wardoff-log-tail-{name}-{unique}"));
        fs::create_dir_all(root.join("Wardoff").join("logs")).expect("create log fixture root");
        Self { root }
    }

    fn log_path(&self) -> PathBuf {
        self.root.join("Wardoff").join("logs").join("wardoff.jsonl")
    }

    fn write_log(&self, name: &str, contents: &str) {
        let path = if name == "wardoff.jsonl" {
            self.log_path()
        } else {
            self.log_path()
                .parent()
                .expect("log path should have a parent")
                .join(name)
        };
        fs::write(path, contents).expect("write log fixture");
    }
}

impl Drop for TempFixture {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.root);
    }
}

fn run_cli(fixture: &TempFixture, args: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_wardoff"))
        .args(args)
        .env("LOCALAPPDATA", &fixture.root)
        .output()
        .expect("run Wardoff CLI")
}

fn lines(contents: impl IntoIterator<Item = String>) -> String {
    contents
        .into_iter()
        .map(|line| format!("{line}\r\n"))
        .collect()
}

#[test]
fn tail_above_limit_exits_one_without_panicking() {
    let fixture = TempFixture::new("invalid");

    for value in [usize::MAX.to_string(), "100001".to_string()] {
        let output = run_cli(&fixture, &["--log", "--tail", &value]);

        assert_eq!(output.status.code(), Some(1));
        assert!(String::from_utf8_lossy(&output.stderr).contains("--tail"));
        assert!(String::from_utf8_lossy(&output.stderr).contains("100000"));
        assert!(!String::from_utf8_lossy(&output.stderr).contains("panicked"));
    }
}

#[test]
fn zero_tail_succeeds_with_empty_output() {
    let fixture = TempFixture::new("zero");

    let output = run_cli(&fixture, &["--log", "--tail", "0"]);

    assert_eq!(output.status.code(), Some(0));
    assert!(output.stdout.is_empty());
}

#[test]
fn tail_limit_and_default_tail_return_expected_records() {
    let all_fixture = TempFixture::new("limit");
    all_fixture.write_log(
        "wardoff.jsonl",
        &lines((1..=3).map(|index| format!("record-{index}"))),
    );

    let all = run_cli(&all_fixture, &["--log", "--tail", "100000"]);
    assert_eq!(all.status.code(), Some(0));
    assert_eq!(String::from_utf8_lossy(&all.stdout).lines().count(), 3);

    let default_fixture = TempFixture::new("default");
    default_fixture.write_log(
        "wardoff.jsonl",
        &lines((1..=25).map(|index| format!("record-{index}"))),
    );
    let default = run_cli(&default_fixture, &["--log"]);
    assert_eq!(default.status.code(), Some(0));
    let default_output = String::from_utf8_lossy(&default.stdout);
    let default_lines: Vec<_> = default_output.lines().collect();
    assert_eq!(default_lines.len(), 20);
    assert_eq!(default_lines[0], "record-6");
    assert_eq!(default_lines[19], "record-25");
}

#[test]
fn rotated_crlf_logs_are_returned_in_chronological_order() {
    let fixture = TempFixture::new("rotation");
    fixture.write_log("wardoff.2.jsonl", &lines(["old-1".into(), "old-2".into()]));
    fixture.write_log("wardoff.1.jsonl", &lines(["mid-1".into(), "mid-2".into()]));
    fixture.write_log("wardoff.jsonl", &lines(["new-1".into(), "new-2".into()]));

    let output = run_cli(&fixture, &["--log", "--tail", "4"]);

    assert_eq!(output.status.code(), Some(0));
    let output_text = String::from_utf8_lossy(&output.stdout);
    let output_lines: Vec<_> = output_text.lines().collect();
    assert_eq!(output_lines, ["mid-1", "mid-2", "new-1", "new-2"]);
}
