use serde_json::Value;
use std::{
    fs,
    io::Write,
    path::PathBuf,
    process::{Command, Output, Stdio},
};
use tempfile::TempDir;

struct Fixture {
    dir: TempDir,
}

impl Fixture {
    fn new() -> Self {
        Self {
            dir: tempfile::tempdir().unwrap(),
        }
    }

    fn command(&self) -> Command {
        let mut command = Command::new(env!("CARGO_BIN_EXE_cpyref"));
        command
            .current_dir(self.dir.path())
            .env("CPYREF_STORE", self.dir.path().join("store"));
        for key in [
            "CPYREF_SCOPE",
            "CPYREF_NAMESPACE",
            "CPYREF_PROJECT",
            "CPYREF_CONFIG",
        ] {
            command.env_remove(key);
        }
        command.env("CPYREF_NO_CONFIG", "true");
        command
    }

    fn run(&self, args: &[&str]) -> Output {
        self.command().args(args).output().unwrap()
    }

    fn store_path(&self) -> PathBuf {
        let output = self.run(&["context"]);
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        let context: Value = serde_json::from_slice(&output.stdout).unwrap();
        PathBuf::from(context["store_path"].as_str().unwrap())
    }

    fn copy(&self, data: &[u8], extra: &[&str]) -> String {
        fs::write(self.dir.path().join("input"), data).unwrap();
        let mut args = vec!["copy", "input"];
        args.extend_from_slice(extra);
        let output = self.run(&args);
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        assert!(output.stderr.is_empty());
        let id = String::from_utf8(output.stdout).unwrap();
        assert!(id.ends_with('\n'));
        let id = id.trim().to_owned();
        assert_eq!(id.len(), 13);
        assert!(id.starts_with('c'));
        id
    }
}

fn failure(output: Output, exit: i32, code: &str) -> Value {
    assert_eq!(
        output.status.code(),
        Some(exit),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(output.stdout.is_empty());
    let error: Value = serde_json::from_slice(&output.stderr).unwrap();
    assert_eq!(error["ok"], false);
    assert_eq!(error["error"]["code"], code);
    error
}

#[test]
fn snapshot_survives_source_modification_and_deletion() {
    let f = Fixture::new();
    let id = f.copy(b"original\r\n\0\xff", &[]);
    fs::write(f.dir.path().join("input"), b"changed").unwrap();
    fs::remove_file(f.dir.path().join("input")).unwrap();
    let output = f.run(&["paste", &id]);
    assert!(output.status.success());
    assert_eq!(output.stdout, b"original\r\n\0\xff");
    assert!(output.stderr.is_empty());
}

#[test]
fn copy_success_is_only_one_reference_line() {
    let f = Fixture::new();
    let id = f.copy(b"prefix secret suffix", &["--text", "secret"]);
    assert_eq!(f.run(&["paste", &id]).stdout, b"secret");
}

#[test]
fn ambiguous_copy_has_no_files_or_stdout() {
    let f = Fixture::new();
    fs::write(f.dir.path().join("input"), b"same same").unwrap();
    let error = failure(f.run(&["copy", "input", "--text", "same"]), 3, "AMBIGUOUS");
    assert_eq!(error["error"]["details"]["match_count"], 2);
    assert!(!f.dir.path().join("store").exists());
}

#[test]
fn missing_copy_has_no_reference() {
    let f = Fixture::new();
    fs::write(f.dir.path().join("input"), b"abc").unwrap();
    failure(f.run(&["copy", "input", "--text", "xyz"]), 2, "NO_MATCH");
}

#[test]
fn missing_file_is_not_a_no_match() {
    let f = Fixture::new();
    failure(f.run(&["copy", "missing"]), 5, "FILE_NOT_FOUND");
}

#[test]
fn parser_errors_use_json_and_distinct_exit_code() {
    let f = Fixture::new();
    failure(f.run(&["copy", "--unknown-option"]), 4, "INVALID_ARGUMENT");
    failure(
        f.run(&["paste", "anything", "--force"]),
        4,
        "INVALID_ARGUMENT",
    );
}

#[test]
fn json_success_and_info_do_not_expand_payload() {
    let f = Fixture::new();
    fs::write(f.dir.path().join("input"), b"TOP_SECRET_PAYLOAD").unwrap();
    let output = f.run(&["copy", "input", "--json"]);
    assert!(output.status.success());
    let value: Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(value["context"]["scope"], "project");
    let id = value["reference"]["id"].as_str().unwrap();
    let info = f.run(&["info", id]);
    assert!(info.status.success());
    assert!(!String::from_utf8_lossy(&info.stdout).contains("TOP_SECRET_PAYLOAD"));
    let metadata: Value = serde_json::from_slice(&info.stdout).unwrap();
    assert_eq!(metadata["bytes"], 18);
    assert_eq!(metadata["sha256"].as_str().unwrap().len(), 64);
}

#[test]
fn stdin_preserves_raw_bytes() {
    let f = Fixture::new();
    let mut child = f
        .command()
        .args(["copy", "-"])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    child
        .stdin
        .take()
        .unwrap()
        .write_all(b"from stdin\r\n\0")
        .unwrap();
    let result = child.wait_with_output().unwrap();
    assert!(
        result.status.success(),
        "{}",
        String::from_utf8_lossy(&result.stderr)
    );
    let id = String::from_utf8(result.stdout).unwrap();
    assert_eq!(f.run(&["paste", id.trim()]).stdout, b"from stdin\r\n\0");
}

#[test]
fn implicit_piped_input_supports_binary_selection_notes_and_preview() {
    let f = Fixture::new();
    let pipe = |args: &[&str], bytes: &[u8]| {
        let mut child = f
            .command()
            .args(args)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .unwrap();
        child.stdin.take().unwrap().write_all(bytes).unwrap();
        child.wait_with_output().unwrap()
    };
    let preview = pipe(
        &["copy", "--lines", "2:2", "--dry-run"],
        b"one\r\n\xff\0two\r\nend",
    );
    assert!(preview.status.success());
    assert!(!f.dir.path().join("store").exists());
    let out = pipe(
        &["copy", "--lines", "2:2", "--note", "piped"],
        b"one\r\n\xff\0two\r\nend",
    );
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    let id = String::from_utf8(out.stdout).unwrap();
    assert_eq!(f.run(&["paste", id.trim()]).stdout, b"\xff\0two\r\n");
    let info: Value = serde_json::from_slice(&f.run(&["info", id.trim()]).stdout).unwrap();
    assert_eq!(info["source"], "<stdin>");
    assert_eq!(info["note"], "piped");
    let empty = pipe(&["copy"], b"");
    assert!(!empty.status.success());
    assert!(empty.stdout.is_empty());
    let error: Value = serde_json::from_slice(&empty.stderr).unwrap();
    assert_eq!(error["error"]["code"], "EMPTY_SELECTION");
}

#[test]
fn output_is_not_overwritten_without_force() {
    let f = Fixture::new();
    let id = f.copy(b"replacement", &[]);
    fs::write(f.dir.path().join("output"), b"existing").unwrap();
    failure(
        f.run(&["paste", &id, "--output", "output"]),
        5,
        "OUTPUT_EXISTS",
    );
    assert_eq!(fs::read(f.dir.path().join("output")).unwrap(), b"existing");
    let result = f.run(&["paste", &id, "--output", "output", "--force"]);
    assert!(result.status.success());
    assert!(result.stdout.is_empty());
    assert_eq!(
        fs::read(f.dir.path().join("output")).unwrap(),
        b"replacement"
    );
}

#[test]
fn corruption_is_detected_before_any_paste_output() {
    let f = Fixture::new();
    let id = f.copy(b"payload", &[]);
    let path = f.store_path().join(format!("{id}.ref"));
    let mut bytes = fs::read(&path).unwrap();
    *bytes.last_mut().unwrap() ^= 1;
    fs::write(path, bytes).unwrap();
    failure(f.run(&["paste", &id]), 7, "CORRUPT_REF");
    assert!(f.run(&["remove", &id]).status.success());
}

#[test]
fn truncated_and_appended_records_are_rejected() {
    for append in [true, false] {
        let f = Fixture::new();
        let id = f.copy(b"payload", &[]);
        let path = f.store_path().join(format!("{id}.ref"));
        let mut bytes = fs::read(&path).unwrap();
        if append {
            bytes.push(0);
        } else {
            bytes.truncate(4);
        }
        fs::write(path, bytes).unwrap();
        failure(f.run(&["paste", &id]), 7, "CORRUPT_REF");
    }
}

#[test]
fn input_limit_prevents_snapshot_creation() {
    let f = Fixture::new();
    fs::write(f.dir.path().join("input"), b"12345").unwrap();
    failure(
        f.run(&["copy", "input", "--max-bytes", "4"]),
        9,
        "SIZE_LIMIT",
    );
    assert!(!f.dir.path().join("store").exists());
}

#[test]
fn full_references_are_required_and_traversal_is_rejected() {
    let f = Fixture::new();
    for id in ["../input", "c", "c123", "C123456789012", "czzzzzzzzzzzz"] {
        failure(f.run(&["paste", id]), 4, "INVALID_REF");
    }
    failure(f.run(&["paste", "c123456789012"]), 6, "REF_NOT_FOUND");
}

#[test]
fn removal_only_affects_the_requested_reference() {
    let f = Fixture::new();
    let first = f.copy(b"first", &[]);
    let second = f.copy(b"second", &[]);
    assert!(f.run(&["remove", &first]).status.success());
    failure(f.run(&["paste", &first]), 6, "REF_NOT_FOUND");
    assert_eq!(f.run(&["paste", &second]).stdout, b"second");
}

#[test]
fn concurrent_processes_publish_complete_distinct_snapshots() {
    let f = Fixture::new();
    fs::write(f.dir.path().join("input"), b"parallel payload\r\n").unwrap();
    let children: Vec<_> = (0..12)
        .map(|_| {
            f.command()
                .args(["copy", "input"])
                .stdout(Stdio::piped())
                .stderr(Stdio::piped())
                .spawn()
                .unwrap()
        })
        .collect();
    let mut ids = std::collections::HashSet::new();
    for child in children {
        let result = child.wait_with_output().unwrap();
        assert!(
            result.status.success(),
            "{}",
            String::from_utf8_lossy(&result.stderr)
        );
        let id = String::from_utf8(result.stdout).unwrap().trim().to_owned();
        assert!(ids.insert(id.clone()));
        assert_eq!(f.run(&["paste", &id]).stdout, b"parallel payload\r\n");
    }
}

#[test]
fn help_succeeds_without_creating_a_store() {
    let f = Fixture::new();
    let output = f.run(&["--help"]);
    assert!(output.status.success());
    assert!(output.stderr.is_empty());
    assert!(String::from_utf8_lossy(&output.stdout).contains("Usage:"));
    assert!(!f.dir.path().join("store").exists());
}

#[cfg(unix)]
#[test]
fn store_has_private_permissions_and_rejects_symlinks() {
    use std::os::unix::fs::{symlink, PermissionsExt};
    let f = Fixture::new();
    let id = f.copy(b"private", &[]);
    let store = f.store_path();
    assert_eq!(
        fs::metadata(&store).unwrap().permissions().mode() & 0o777,
        0o700
    );
    let path = store.join(format!("{id}.ref"));
    assert_eq!(
        fs::metadata(&path).unwrap().permissions().mode() & 0o777,
        0o600
    );
    fs::remove_file(&path).unwrap();
    symlink(f.dir.path().join("input"), &path).unwrap();
    failure(f.run(&["paste", &id]), 7, "CORRUPT_REF");
}

#[cfg(unix)]
#[test]
fn output_symlink_is_not_followed_even_with_force() {
    use std::os::unix::fs::symlink;
    let f = Fixture::new();
    let id = f.copy(b"replacement", &[]);
    let target = f.dir.path().join("target");
    fs::write(&target, b"protected").unwrap();
    symlink(&target, f.dir.path().join("output")).unwrap();
    failure(
        f.run(&["paste", &id, "--output", "output", "--force"]),
        5,
        "UNSAFE_OUTPUT",
    );
    assert_eq!(fs::read(target).unwrap(), b"protected");
}
