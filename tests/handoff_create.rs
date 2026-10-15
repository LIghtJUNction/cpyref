use serde_json::Value;
use std::{
    fs,
    io::Write,
    path::PathBuf,
    process::{Command, Output, Stdio},
};
use tempfile::TempDir;

struct Fixture(TempDir);

impl Fixture {
    fn new() -> Self {
        let fixture = Self(tempfile::tempdir().unwrap());
        fs::create_dir(fixture.path(".git")).unwrap();
        fixture
    }

    fn path(&self, name: &str) -> PathBuf {
        self.0.path().join(name)
    }

    fn command(&self, args: &[&str]) -> Command {
        let mut command = Command::new(env!("CARGO_BIN_EXE_cpyref"));
        for (key, _) in std::env::vars_os() {
            if key.to_str().is_some_and(|key| key.starts_with("CPYREF_")) {
                command.env_remove(key);
            }
        }
        command
            .current_dir(self.0.path())
            .env("CPYREF_NO_CONFIG", "true")
            .env("CPYREF_STORE", self.path("store"))
            .args(args);
        command
    }

    fn run(&self, args: &[&str]) -> Output {
        self.command(args).output().unwrap()
    }

    fn json(&self, args: &[&str]) -> Value {
        good(self.run(args))
    }

    fn input(&self, args: &[&str], bytes: &[u8]) -> Output {
        let mut child = self
            .command(args)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .unwrap();
        child.stdin.take().unwrap().write_all(bytes).unwrap();
        child.wait_with_output().unwrap()
    }

    fn assert_no_store(&self) {
        assert!(!self.path("store").exists());
    }
}

fn good(output: Output) -> Value {
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(output.stderr.is_empty());
    serde_json::from_slice(&output.stdout).unwrap()
}

fn bad(output: Output, code: &str) -> Value {
    assert!(!output.status.success());
    assert!(output.stdout.is_empty());
    let report: Value = serde_json::from_slice(&output.stderr).unwrap();
    assert_eq!(report["error"]["code"], code, "{report}");
    report
}

#[test]
fn one_command_handoff_retains_exact_notes_and_files_after_sources_disappear() {
    let fixture = Fixture::new();
    let notes = b"next step\r\n\0\xff";
    let patch = b"diff\0\xfe\r\n";
    fs::write(fixture.path("handoff.md"), notes).unwrap();
    fs::write(fixture.path("change=1.patch"), patch).unwrap();
    let report = fixture.json(&[
        "handoff",
        "--input",
        "handoff.md",
        "--attach",
        "patch=change=1.patch",
        "--format",
        "json",
    ]);
    assert_eq!(report["created"], true);
    assert_eq!(report["kind"], "bundle");
    let token = report["token"].as_str().unwrap();
    fs::remove_file(fixture.path("handoff.md")).unwrap();
    fs::remove_file(fixture.path("change=1.patch")).unwrap();
    let index = fixture.json(&["receive", token, "--list"]);
    assert_eq!(index["item_count"], 2);
    let notes_id = index["items"]
        .as_array()
        .unwrap()
        .iter()
        .find(|item| item["name"] == "notes")
        .unwrap()["reference"]
        .as_str()
        .unwrap();
    let metadata = fixture.json(&["info", notes_id]);
    let original_source = fs::canonicalize(fixture.0.path())
        .unwrap()
        .join("handoff.md");
    assert_eq!(
        metadata["source"],
        original_source.to_string_lossy().as_ref()
    );
    assert_eq!(metadata["span"]["selector"], "whole");
    assert_eq!(metadata["span"]["byte_start"], 0);
    assert_eq!(metadata["span"]["byte_end"], notes.len());
    for (name, output, expected) in [
        ("notes", "restored-notes", notes.as_slice()),
        ("patch", "restored-patch", patch.as_slice()),
    ] {
        let output_status = fixture.run(&["receive", token, "--item", name, "--output", output]);
        assert!(
            output_status.status.success(),
            "{}",
            String::from_utf8_lossy(&output_status.stderr)
        );
        assert_eq!(fs::read(fixture.path(output)).unwrap(), expected);
    }
}

#[test]
fn explicit_stdin_notes_create_a_single_item_bundle() {
    let fixture = Fixture::new();
    let notes = b"stdin notes\r\n\0\xff";
    let report = good(fixture.input(&["handoff", "--input", "-", "--format", "json"], notes));
    let token = report["token"].as_str().unwrap();
    let index = fixture.json(&["receive", token, "--list"]);
    assert_eq!(index["item_count"], 1);
    assert_eq!(index["items"][0]["name"], "notes");
    let reference = index["items"][0]["reference"].as_str().unwrap();
    assert_eq!(fixture.json(&["info", reference])["source"], "<stdin>");
    let received = fixture.run(&["receive", token, "--item", "notes"]);
    assert!(received.status.success());
    assert_eq!(received.stdout, notes);
}

#[test]
fn attachment_names_and_paths_are_checked_before_store_creation() {
    let fixture = Fixture::new();
    fs::write(fixture.path("notes"), b"notes").unwrap();
    fs::write(fixture.path("patch"), b"patch").unwrap();
    let long_name = format!("{}=patch", "a".repeat(65));
    for attachment in [
        "missing-equals",
        "=patch",
        "notes=patch",
        "../patch=patch",
        ".patch=patch",
        "a/b=patch",
        "中文=patch",
        "patch=-",
        "patch=",
        &long_name,
    ] {
        bad(
            fixture.run(&["handoff", "--input", "notes", "--attach", attachment]),
            "INVALID_ARGUMENT",
        );
        fixture.assert_no_store();
    }
    bad(
        fixture.run(&[
            "handoff",
            "--input",
            "notes",
            "--attach",
            "patch=patch",
            "--attach",
            "patch=patch",
        ]),
        "INVALID_ARGUMENT",
    );
    fixture.assert_no_store();
}

#[test]
fn missing_later_attachment_does_not_save_earlier_inputs() {
    let fixture = Fixture::new();
    fs::write(fixture.path("notes"), b"notes").unwrap();
    fs::write(fixture.path("patch"), b"patch").unwrap();
    bad(
        fixture.run(&[
            "handoff",
            "--input",
            "notes",
            "--attach",
            "patch=patch",
            "--attach",
            "missing=absent",
        ]),
        "FILE_NOT_FOUND",
    );
    fixture.assert_no_store();
}

#[test]
fn directories_and_empty_inputs_do_not_create_a_store() {
    let fixture = Fixture::new();
    fs::write(fixture.path("notes"), b"notes").unwrap();
    fs::write(fixture.path("empty"), b"").unwrap();
    fs::create_dir(fixture.path("directory")).unwrap();
    for (args, code) in [
        (vec!["handoff", "--input", "empty"], "EMPTY_SELECTION"),
        (
            vec!["handoff", "--input", "notes", "--attach", "empty=empty"],
            "EMPTY_SELECTION",
        ),
        (
            vec!["handoff", "--input", "directory"],
            "UNSUPPORTED_SOURCE",
        ),
        (
            vec!["handoff", "--input", "notes", "--attach", "dir=directory"],
            "UNSUPPORTED_SOURCE",
        ),
    ] {
        bad(fixture.run(&args), code);
        fixture.assert_no_store();
    }
}

#[test]
fn aggregate_byte_budget_includes_every_attachment_and_accepts_exact_limit() {
    let fixture = Fixture::new();
    fs::write(fixture.path("notes"), b"123").unwrap();
    fs::write(fixture.path("patch"), b"4567").unwrap();
    let error = bad(
        fixture.run(&[
            "handoff",
            "--input",
            "notes",
            "--attach",
            "patch=patch",
            "--max-bytes",
            "6",
        ]),
        "SIZE_LIMIT",
    );
    assert_eq!(error["error"]["details"]["limit"], 6);
    fixture.assert_no_store();
    let report = fixture.json(&[
        "handoff",
        "--input",
        "notes",
        "--attach",
        "patch=patch",
        "--max-bytes",
        "7",
        "--format",
        "json",
    ]);
    assert_eq!(report["created"], true);
    assert_eq!(fixture.json(&["stats"])["count"], 3);
}

#[test]
fn invalid_byte_limits_are_rejected_before_store_creation() {
    let fixture = Fixture::new();
    fs::write(fixture.path("notes"), b"notes").unwrap();
    for limit in ["0", "134217729"] {
        bad(
            fixture.run(&["handoff", "--input", "notes", "--max-bytes", limit]),
            "INVALID_ARGUMENT",
        );
        fixture.assert_no_store();
    }
}

#[test]
fn capacity_preflight_includes_inputs_and_the_bundle_without_creating_store() {
    let fixture = Fixture::new();
    fs::write(fixture.path("notes"), b"notes").unwrap();
    fs::write(fixture.path("patch"), b"patch").unwrap();
    let error = bad(
        fixture.run(&[
            "--max-entries",
            "2",
            "handoff",
            "--input",
            "notes",
            "--attach",
            "patch=patch",
        ]),
        "ENTRY_LIMIT",
    );
    assert_eq!(error["error"]["details"]["additional_entries"], 3);
    fixture.assert_no_store();
    fixture.json(&[
        "--max-entries",
        "3",
        "handoff",
        "--input",
        "notes",
        "--attach",
        "patch=patch",
        "--format",
        "json",
    ]);
    assert_eq!(fixture.json(&["stats"])["count"], 3);
}

#[test]
fn capacity_preflight_preserves_existing_references() {
    let fixture = Fixture::new();
    fs::write(fixture.path("notes"), b"notes").unwrap();
    let previous = fixture.run(&["copy", "notes"]);
    assert!(previous.status.success());
    let reference = String::from_utf8(previous.stdout).unwrap();
    let error = bad(
        fixture.run(&["--max-entries", "2", "handoff", "--input", "notes"]),
        "ENTRY_LIMIT",
    );
    assert_eq!(error["error"]["details"]["count"], 1);
    assert_eq!(fixture.json(&["stats"])["count"], 1);
    let old = fixture.run(&["paste", reference.trim()]);
    assert!(old.status.success());
    assert_eq!(old.stdout, b"notes");
}

#[test]
fn too_many_attachments_are_rejected_without_reading_files_or_creating_store() {
    let fixture = Fixture::new();
    let mut args = vec![
        "handoff".to_owned(),
        "--input".to_owned(),
        "absent".to_owned(),
    ];
    for index in 0..100 {
        args.extend(["--attach".to_owned(), format!("item{index}=absent")]);
    }
    let args: Vec<_> = args.iter().map(String::as_str).collect();
    bad(fixture.run(&args), "INVALID_ARGUMENT");
    fixture.assert_no_store();
}
