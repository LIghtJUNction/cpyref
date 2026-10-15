use serde_json::Value;
use std::{
    fs,
    path::{Path, PathBuf},
    process::{Command, Output},
};
use tempfile::TempDir;

struct Fixture(TempDir);
impl Fixture {
    fn new() -> Self {
        let f = Self(tempfile::tempdir().unwrap());
        fs::create_dir(f.0.path().join(".git")).unwrap();
        fs::create_dir(f.0.path().join("src")).unwrap();
        fs::write(f.0.path().join("src/a.rs"), b"fn a() { original(); }\r\n").unwrap();
        fs::write(f.0.path().join("src/b.rs"), b"fn b() {}\n").unwrap();
        f
    }
    fn command(&self) -> Command {
        let mut c = Command::new(env!("CARGO_BIN_EXE_cpyref"));
        c.current_dir(self.0.path());
        for key in [
            "CPYREF_SCOPE",
            "CPYREF_NAMESPACE",
            "CPYREF_CONFIG",
            "CPYREF_PROJECT",
            "CPYREF_MAX_ENTRIES",
        ] {
            c.env_remove(key);
        }
        c.env("CPYREF_NO_CONFIG", "true")
            .env("CPYREF_STORE", self.0.path().join("store"));
        c
    }
    fn run(&self, args: &[&str]) -> Output {
        self.command().args(args).output().unwrap()
    }
    fn json(&self, args: &[&str]) -> Value {
        success(self.run(args))
    }
    fn copy(&self, file: &str, extra: &[&str]) -> String {
        let o = self
            .command()
            .args(["copy", file])
            .args(extra)
            .output()
            .unwrap();
        assert!(o.status.success(), "{}", String::from_utf8_lossy(&o.stderr));
        String::from_utf8(o.stdout).unwrap().trim().to_owned()
    }
    fn child_run(&self, extra: &[&str], child: &[&str]) -> Output {
        self.command()
            .arg("run")
            .args(extra)
            .arg("--")
            .arg(env!("CARGO_BIN_EXE_cpyref"))
            .args(child)
            .output()
            .unwrap()
    }
    fn source(&self, name: &str) -> PathBuf {
        self.0.path().join(name)
    }
}
fn success(o: Output) -> Value {
    assert!(o.status.success(), "{}", String::from_utf8_lossy(&o.stderr));
    assert!(
        o.stderr.is_empty(),
        "{}",
        String::from_utf8_lossy(&o.stderr)
    );
    serde_json::from_slice(&o.stdout).unwrap()
}
fn failure(o: Output, code: &str) -> Value {
    assert!(!o.status.success());
    assert!(o.stdout.is_empty());
    let v: Value = serde_json::from_slice(&o.stderr).unwrap();
    assert_eq!(v["error"]["code"], code);
    v
}
fn run_reference(v: &Value) -> &str {
    v["reference"].as_str().unwrap()
}

#[test]
fn verify_checks_original_offsets_moved_duplicates_missing_and_binary() {
    let f = Fixture::new();
    fs::write(f.source("binary"), b"AA\0\xffBB").unwrap();
    let id = f.copy("binary", &[]);
    assert_eq!(f.json(&["verify", &id])["status"], "original_match");
    fs::write(f.source("binary"), b"prefixAA\0\xffBB").unwrap();
    let o = f.run(&["verify", &id]);
    assert_eq!(o.status.code(), Some(10));
    let v: Value = serde_json::from_slice(&o.stdout).unwrap();
    assert_eq!(v["status"], "moved_unique");
    assert_eq!(v["candidates"][0]["byte_start"], 6);
    fs::write(f.source("binary"), b"prefixAA\0\xffBBAA\0\xffBB").unwrap();
    let v: Value = serde_json::from_slice(&f.run(&["verify", &id]).stdout).unwrap();
    assert_eq!(v["status"], "multiple_matches");
    assert_eq!(v["match_count"], 2);
    fs::write(f.source("binary"), b"different").unwrap();
    let v: Value = serde_json::from_slice(&f.run(&["verify", &id]).stdout).unwrap();
    assert_eq!(v["status"], "changed");
    fs::remove_file(f.source("binary")).unwrap();
    let v: Value = serde_json::from_slice(&f.run(&["verify", &id]).stdout).unwrap();
    assert_eq!(v["status"], "source_missing");
}

#[test]
fn verify_unrelated_edits_do_not_invalidate_and_overlaps_are_counted() {
    let f = Fixture::new();
    fs::write(f.source("input"), b"banana\nother").unwrap();
    let id = f.copy("input", &["--text", "ana", "--occurrence", "1"]);
    fs::write(f.source("input"), b"banana\nchanged elsewhere").unwrap();
    let v = f.json(&["verify", &id]);
    assert_eq!(v["status"], "original_match");
    assert_eq!(v["match_count"], 2);
    assert_eq!(v["semantic_position_verified"], false);
    fs::write(f.source("input"), b"xbanana\n").unwrap();
    let o = f.run(&["verify", &id]);
    assert_eq!(o.status.code(), Some(10));
    let v: Value = serde_json::from_slice(&o.stdout).unwrap();
    assert_eq!(v["status"], "multiple_matches");
}

#[test]
fn snapshot_checks_exact_selected_files_and_reports_additions_and_deletions() {
    let f = Fixture::new();
    let first = f.json(&["snapshot", "src"]);
    let id = run_reference(&first);
    assert_eq!(first["file_count"], 2);
    assert_eq!(first["content_saved"], false);
    assert_eq!(f.json(&["snapshot", "check", id])["status"], "unchanged");
    fs::write(f.source("unselected"), b"not part of selection").unwrap();
    assert_eq!(f.json(&["snapshot", "check", id])["changed_count"], 0);
    fs::write(f.source("src/a.rs"), b"fn a() { changed(); }\r\n").unwrap();
    fs::remove_file(f.source("src/b.rs")).unwrap();
    fs::write(f.source("src/c.rs"), b"fn c() {}\n").unwrap();
    let o = f.run(&["snapshot", "check", id, "--limit", "1"]);
    assert_eq!(o.status.code(), Some(10));
    let v: Value = serde_json::from_slice(&o.stdout).unwrap();
    assert_eq!(v["changed_count"], 3);
    assert_eq!(v["omitted"], 2);
    assert_eq!(
        v["counts"],
        serde_json::json!({"added":1,"removed":1,"modified":1})
    );
    let second = f.json(&["snapshot", "src"]);
    let o = f.run(&["snapshot", "diff", id, run_reference(&second)]);
    assert_eq!(o.status.code(), Some(10));
    let v: Value = serde_json::from_slice(&o.stdout).unwrap();
    assert_eq!(v["changed_count"], 3);
    let shown = f.json(&["snapshot", "show", id, "--limit", "1"]);
    assert_eq!(shown["files"].as_array().unwrap().len(), 1);
    assert_eq!(shown["omitted"], 1);
    assert!(!shown.to_string().contains("original()"));
}

#[test]
fn snapshot_explicit_files_are_deduplicated_and_store_and_git_are_excluded() {
    let f = Fixture::new();
    let id = f.copy("src/a.rs", &[]);
    let v = f.json(&["snapshot", ".", "src/a.rs"]);
    assert_eq!(v["file_count"], 2);
    assert_eq!(
        f.json(&["snapshot", "check", run_reference(&v)])["changed_count"],
        0
    );
    failure(f.run(&["snapshot"]), "INVALID_ARGUMENT");
    failure(f.run(&["snapshot", "check", &id]), "WRONG_KIND");
    failure(f.run(&["snapshot", "missing"]), "FILE_NOT_FOUND");
    let outside = tempfile::tempdir().unwrap();
    fs::write(outside.path().join("file"), "external").unwrap();
    failure(
        f.command()
            .arg("snapshot")
            .arg(fs::canonicalize(outside.path().join("file")).unwrap())
            .output()
            .unwrap(),
        "INVALID_ARGUMENT",
    );
}

#[test]
fn run_retains_both_streams_and_preserves_child_exit() {
    let f = Fixture::new();
    let result = success(f.child_run(&[], &["--version"]));
    let id = run_reference(&result);
    assert_eq!(result["command_exit"], 0);
    assert_eq!(result["analysis"]["counts"], Value::Null);
    assert_eq!(result["analysis"]["parse_status"], "unknown");
    assert_eq!(f.json(&["logs", id])["reference"], id);
    let raw = f.json(&["logs", id, "--stream", "stdout", "--max-bytes", "4"]);
    assert_eq!(raw["content"], "cpyr");
    assert_eq!(raw["truncated"], true);
    let exported = f.json(&["logs", id, "--stream", "stdout", "--output", "exported"]);
    assert_eq!(exported["complete"], true);
    assert_eq!(
        fs::read(f.source("exported")).unwrap(),
        f.run(&["--version"]).stdout
    );
    failure(
        f.run(&["logs", id, "--stream", "stdout", "--output", "exported"]),
        "OUTPUT_EXISTS",
    );
    failure(f.run(&["paste", id]), "WRONG_KIND");
    let o = f.child_run(&[], &["paste", "invalid"]);
    assert_eq!(o.status.code(), Some(4));
    assert!(o.stderr.is_empty());
    let v: Value = serde_json::from_slice(&o.stdout).unwrap();
    assert_eq!(v["command_exit"], 4);
    let raw = f.json(&["logs", run_reference(&v), "--stream", "stderr"]);
    assert!(raw["content"].as_str().unwrap().contains("INVALID_REF"));
}

#[test]
fn log_tail_reads_only_the_selected_stream_and_rejects_conflicting_modes() {
    let f = Fixture::new();
    fs::write(f.source("log"), b"many lines\r\nfinish\0\xff\r\n").unwrap();
    let bytes = f.copy("log", &[]);
    let result = success(f.child_run(&[], &["paste", &bytes]));
    let id = run_reference(&result);
    let tail = f.json(&[
        "logs",
        id,
        "--stream",
        "stdout",
        "--tail",
        "--max-bytes",
        "4",
    ]);
    assert_eq!(tail["content"], "\0\u{fffd}\r\n");
    assert_eq!(tail["bytes"], 4);
    assert_eq!(tail["next_offset"], tail["total_bytes"]);
    assert_eq!(tail["truncated"], false);
    let empty = f.json(&["logs", id, "--stream", "stderr", "--tail"]);
    assert_eq!(empty["bytes"], 0);
    assert_eq!(empty["offset"], 0);
    assert_eq!(empty["content"], "");
    assert_eq!(empty["truncated"], false);
    for flags in [
        vec!["--offset", "0"],
        vec!["--failed"],
        vec!["--check"],
        vec!["--view", "summary"],
        vec!["--output", "no-export"],
    ] {
        let mut args = vec!["logs", id, "--stream", "stdout", "--tail"];
        args.extend(flags);
        failure(f.run(&args), "INVALID_ARGUMENT");
    }
    failure(f.run(&["logs", id, "--tail"]), "INVALID_ARGUMENT");
    assert!(!f.source("no-export").exists());
}

#[test]
fn cargo_parser_separates_skips_and_keeps_failure_context() {
    let f = Fixture::new();
    let log = "running 3 tests\ntest pass ... ok\ntest skip ... ignored\ntest bad ... FAILED\n\n---- bad stdout ----\nthread 'bad' panicked at src/a.rs:12:3:\nexpected 1, got 2\n\ntest result: FAILED. 1 passed; 1 failed; 1 ignored; 0 measured; 0 filtered out; finished in 0.01s\n";
    fs::write(f.source("log"), log).unwrap();
    let log_id = f.copy("log", &[]);
    let v = success(f.child_run(&["--parser", "cargo"], &["paste", &log_id]));
    let id = run_reference(&v);
    assert_eq!(
        v["analysis"]["counts"],
        serde_json::json!({"passed":1,"failed":1,"skipped":1})
    );
    assert_eq!(v["command_exit"], 0); // Exit status and parser counts are independent facts.
    let failed = f.json(&["logs", id, "--failed", "--limit", "1"]);
    assert_eq!(failed["analysis"]["failures"].as_array().unwrap().len(), 1);
    assert!(
        failed["analysis"]["failure_locations_omitted"]
            .as_u64()
            .unwrap()
            > 0
    );
    let next = f.json(&["logs", id, "--failed", "--offset", "2"]);
    assert!(next.to_string().contains("src/a.rs:12:3"));
    f.json(&["logs", id, "--stream", "stdout", "--output", "full-log"]);
    assert_eq!(fs::read(f.source("full-log")).unwrap(), log.as_bytes());
}

#[test]
fn incomplete_cargo_output_never_claims_complete_counts() {
    let f = Fixture::new();
    fs::write(f.source("log"), "running 2 tests\ntest first ... ok\n").unwrap();
    let id = f.copy("log", &[]);
    let v = success(f.child_run(&["--parser", "cargo"], &["paste", &id]));
    assert_eq!(v["analysis"]["counts"], Value::Null);
    assert_eq!(v["analysis"]["parse_status"], "unknown");
}

#[test]
fn run_snapshot_dependency_prevents_cleanup_and_gate_checks_current_source() {
    let f = Fixture::new();
    let snapshot = f.json(&["snapshot", "src"]);
    let snapshot_id = run_reference(&snapshot);
    let run = success(f.child_run(
        &["--snapshot", snapshot_id, "--artifact", "src/a.rs"],
        &["--version"],
    ));
    let run_id = run_reference(&run);
    assert_eq!(run["source_after"]["status"], "unchanged");
    assert_eq!(run["artifact_count"], 1);
    failure(f.run(&["remove", snapshot_id]), "REF_IN_USE");
    failure(f.run(&["clean", snapshot_id]), "REF_IN_USE");
    fs::write(f.source("src/a.rs"), "changed").unwrap();
    let bytes = f.copy("src/a.rs", &[]);
    failure(
        f.child_run(
            &["--snapshot", snapshot_id],
            &["paste", &bytes, "--output", "must-not-exist"],
        ),
        "SNAPSHOT_CHANGED",
    );
    assert!(!f.source("must-not-exist").exists());
    let plan = f.json(&["clean", snapshot_id, run_id]);
    f.json(&[
        "clean",
        snapshot_id,
        run_id,
        "--apply",
        "--confirm",
        plan["confirm"].as_str().unwrap(),
    ]);
    assert_eq!(f.json(&["stats"])["count"], 1);
}

#[test]
fn run_detects_changes_during_execution_and_inline_manifests_do_not_add_references() {
    let f = Fixture::new();
    fs::write(f.source("replacement"), "changed by command").unwrap();
    let replace = f.copy("replacement", &[]);
    let v = success(f.child_run(
        &["--input", "src"],
        &["paste", &replace, "--output", "src/a.rs", "--force"],
    ));
    assert_eq!(v["source_after"]["status"], "changed");
    assert_eq!(v["source_after"]["changed_count"], 1);
    assert_eq!(f.json(&["stats"])["count"], 2);
}

#[test]
fn full_store_prevents_start_and_post_command_recording_failure_retains_logs() {
    let f = Fixture::new();
    let id = f.copy("src/a.rs", &[]);
    failure(
        f.child_run(
            &["--max-entries", "1"],
            &["paste", &id, "--output", "must-not-exist"],
        ),
        "ENTRY_LIMIT",
    );
    assert!(!f.source("must-not-exist").exists());
    // A concurrent/new child reference can consume the last slot after preflight.
    let other = Fixture::new();
    let error = failure(
        other.child_run(&["--max-entries", "1"], &["copy", "src/a.rs"]),
        "RECORDING_FAILED",
    );
    assert_eq!(error["error"]["details"]["command_exit"], 0);
    assert_eq!(error["error"]["details"]["cause"]["code"], "ENTRY_LIMIT");
    let retained = error["error"]["details"]["stdout"]["path"]
        .as_str()
        .unwrap();
    assert_eq!(fs::read(retained).unwrap().len(), 14);
    assert_eq!(other.json(&["stats"])["count"], 1);
}

#[test]
fn run_log_integrity_is_checked_before_any_excerpt_or_export() {
    let f = Fixture::new();
    let v = success(f.child_run(&[], &["--version"]));
    let id = run_reference(&v);
    let context = f.json(&["context"]);
    let path = Path::new(context["store_path"].as_str().unwrap()).join(format!("{id}.ref"));
    let mut bytes = fs::read(&path).unwrap();
    *bytes.last_mut().unwrap() ^= 1;
    fs::write(path, bytes).unwrap();
    failure(f.run(&["logs", id]), "CORRUPT_REF");
    failure(
        f.run(&["logs", id, "--stream", "stdout", "--output", "bad-export"]),
        "CORRUPT_REF",
    );
    assert!(!f.source("bad-export").exists());
}

#[test]
fn new_operations_remain_in_the_selected_namespace() {
    let f = Fixture::new();
    let snapshot = f.json(&["snapshot", "src"]);
    let id = run_reference(&snapshot);
    failure(
        f.run(&["--namespace", "other", "snapshot", "show", id]),
        "REF_NOT_FOUND",
    );
    failure(
        f.run(&["--namespace", "other", "handoff", id]),
        "REF_NOT_FOUND",
    );
    let run = success(f.child_run(&[], &["--version"]));
    failure(
        f.run(&["--namespace", "other", "logs", run_reference(&run)]),
        "REF_NOT_FOUND",
    );
}

#[cfg(unix)]
#[test]
fn handoff_shell_command_survives_quotes_and_inherited_configuration() {
    let dir = tempfile::Builder::new()
        .prefix("cpyref space'quote-")
        .tempdir()
        .unwrap();
    let f = Fixture(dir);
    fs::create_dir(f.source(".git")).unwrap();
    fs::write(f.source("input"), b"exact\r\n\0\xff").unwrap();
    let id = f.copy("input", &[]);
    let handoff = f.json(&["handoff", &id]);
    assert_eq!(handoff["transport"], "same_host");
    assert!(handoff["argv"]
        .as_array()
        .unwrap()
        .iter()
        .any(|v| v == "--no-config"));
    let output = f.run(&["handoff", &id, "--format", "sh"]);
    assert!(output.status.success());
    let shell = String::from_utf8(output.stdout).unwrap();
    let pasted = Command::new("sh")
        .arg("-c")
        .arg(shell)
        .env("PATH", "/usr/bin:/bin")
        .env("CPYREF_CONFIG", "/does/not/exist")
        .env("CPYREF_PROJECT", "/does/not/exist")
        .env("CPYREF_NAMESPACE", "wrong")
        .output()
        .unwrap();
    assert!(
        pasted.status.success(),
        "{}",
        String::from_utf8_lossy(&pasted.stderr)
    );
    assert_eq!(pasted.stdout, b"exact\r\n\0\xff");
}

#[cfg(unix)]
#[test]
fn source_symlinks_are_refused_during_capture_and_recheck() {
    use std::os::unix::fs::symlink;
    let f = Fixture::new();
    let s = f.json(&["snapshot", "src"]);
    symlink("src", f.source("shortcut")).unwrap();
    failure(f.run(&["snapshot", "shortcut/../src"]), "UNSAFE_SOURCE");
    failure(f.run(&["snapshot", "missing/../src"]), "FILE_NOT_FOUND");
    fs::rename(f.source("src"), f.source("moved")).unwrap();
    symlink("moved", f.source("src")).unwrap();
    failure(f.run(&["snapshot", "src"]), "UNSAFE_SOURCE");
    failure(
        f.run(&["snapshot", "check", run_reference(&s)]),
        "UNSAFE_SOURCE",
    );
}

#[cfg(unix)]
#[test]
fn signal_termination_is_recorded_separately_from_exit_code() {
    let f = Fixture::new();
    let o = f.run(&[
        "run",
        "--",
        "sh",
        "-c",
        "printf 'before signal'; kill -TERM $$",
    ]);
    assert_eq!(o.status.code(), Some(143));
    assert!(o.stderr.is_empty());
    let v: Value = serde_json::from_slice(&o.stdout).unwrap();
    assert_eq!(v["command_exit"], Value::Null);
    assert_eq!(v["signal"], 15);
    assert_eq!(
        f.json(&["logs", run_reference(&v), "--stream", "stdout"])["content"],
        "before signal"
    );
}

#[test]
fn run_views_expose_source_hashes_artifacts_and_command_with_pagination() {
    let f = Fixture::new();
    let v = success(f.child_run(
        &["--input", "src", "--artifact", "src/a.rs"],
        &["--version"],
    ));
    let id = run_reference(&v);
    let inputs = f.json(&["logs", id, "--view", "inputs", "--limit", "1"]);
    assert_eq!(inputs["file_count"], 2);
    assert_eq!(inputs["omitted"], 1);
    assert_eq!(inputs["files"][0]["sha256"].as_str().unwrap().len(), 64);
    let artifacts = f.json(&["logs", id, "--view", "artifacts"]);
    assert_eq!(artifacts["artifacts"][0]["status"], "observed");
    assert!(artifacts["relationship"]
        .as_str()
        .unwrap()
        .contains("not_proven"));
    let command = f.json(&["logs", id, "--view", "command", "--max-bytes", "4"]);
    assert_eq!(command["argument_count"], 2);
    assert_eq!(command["truncated"], true);
    let next = f.json(&["logs", id, "--view", "command", "--offset", "1"]);
    assert_eq!(next["argv"][0]["value"], "--version");
}

#[test]
fn binary_logs_are_exported_byte_for_byte_with_bounded_previews() {
    let f = Fixture::new();
    let bytes: Vec<u8> = (0..2 * 1024 * 1024).map(|i| (i % 256) as u8).collect();
    fs::write(f.source("binary"), &bytes).unwrap();
    let source = f.copy("binary", &[]);
    let v = success(f.child_run(&[], &["paste", &source]));
    let id = run_reference(&v);
    assert_eq!(v["stdout_bytes"], bytes.len());
    let preview = f.json(&["logs", id, "--stream", "stdout", "--max-bytes", "64"]);
    assert_eq!(preview["bytes"], 64);
    assert_eq!(preview["next_offset"], 64);
    f.json(&["logs", id, "--stream", "stdout", "--output", "exported"]);
    assert_eq!(fs::read(f.source("exported")).unwrap(), bytes);
}

#[cfg(unix)]
#[test]
fn unknown_failures_include_a_tail_and_truncated_context_keeps_the_failure_line() {
    let f = Fixture::new();
    let o = f.run(&[
        "run",
        "--",
        "sh",
        "-c",
        "printf 'unexpected failure detail\\n' >&2; exit 7",
    ]);
    assert_eq!(o.status.code(), Some(7));
    let v: Value = serde_json::from_slice(&o.stdout).unwrap();
    assert!(v["analysis"]["unclassified_failure_tail"]
        .to_string()
        .contains("unexpected failure detail"));
    fs::write(
        f.source("log"),
        "preceding context\nerror: actual failure\n",
    )
    .unwrap();
    let source = f.copy("log", &[]);
    let v = success(f.child_run(&[], &["paste", &source]));
    let excerpts = f.json(&["logs", run_reference(&v), "--failed", "--max-bytes", "20"]);
    assert!(excerpts.to_string().contains("error: actual failur"));
    fs::write(
        f.source("long"),
        format!("{}fatal-tail", "x".repeat(1024 * 1024)),
    )
    .unwrap();
    let o = f.run(&["run", "--", "sh", "-c", "cat long; exit 7"]);
    assert_eq!(o.status.code(), Some(7));
    let v: Value = serde_json::from_slice(&o.stdout).unwrap();
    assert!(v["analysis"]["unclassified_failure_tail"][0]["context"][0]
        .as_str()
        .unwrap()
        .ends_with("fatal-tail"));
}

#[test]
fn run_check_reports_later_source_and_artifact_changes_without_rewriting_old_result() {
    let f = Fixture::new();
    fs::write(f.source("binary-output"), "build v1").unwrap();
    let run = success(f.child_run(
        &["--input", "src", "--artifact", "binary-output"],
        &["--version"],
    ));
    let id = run_reference(&run);
    assert_eq!(f.json(&["logs", id, "--check"])["status"], "unchanged");
    fs::write(f.source("src/a.rs"), "source changed").unwrap();
    fs::write(f.source("binary-output"), "build v2").unwrap();
    let o = f.run(&["logs", id, "--check"]);
    assert_eq!(o.status.code(), Some(10));
    let changes: Value = serde_json::from_slice(&o.stdout).unwrap();
    assert_eq!(changes["changed_count"], 2);
    assert_eq!(changes["historical_command_exit"], 0);
    assert_eq!(f.json(&["logs", id])["source_after"]["status"], "unchanged");
}

#[test]
fn multiple_corrupt_headers_can_still_be_removed_explicitly() {
    let f = Fixture::new();
    let first = f.copy("src/a.rs", &[]);
    let second = f.copy("src/b.rs", &[]);
    let context = f.json(&["context"]);
    for id in [&first, &second] {
        fs::write(
            Path::new(context["store_path"].as_str().unwrap()).join(format!("{id}.ref")),
            b"bad header",
        )
        .unwrap();
    }
    assert!(f.run(&["remove", &first]).status.success());
    assert!(f.run(&["remove", &second]).status.success());
}

#[test]
fn malformed_run_length_is_corruption_rather_than_an_overflow_panic() {
    let f = Fixture::new();
    let run = success(f.child_run(&[], &["--version"]));
    let id = run_reference(&run);
    let context = f.json(&["context"]);
    let path = Path::new(context["store_path"].as_str().unwrap()).join(format!("{id}.ref"));
    let bytes = fs::read(&path).unwrap();
    let len = u32::from_le_bytes(bytes[8..12].try_into().unwrap()) as usize;
    let mut header: Value = serde_json::from_slice(&bytes[12..12 + len]).unwrap();
    header["bytes"] = serde_json::json!(usize::MAX);
    header["span"]["byte_end"] = serde_json::json!(usize::MAX);
    let header = serde_json::to_vec(&header).unwrap();
    let mut rewritten = bytes[..8].to_vec();
    rewritten.extend_from_slice(&(header.len() as u32).to_le_bytes());
    rewritten.extend_from_slice(&header);
    rewritten.extend_from_slice(&bytes[12 + len..]);
    fs::write(path, rewritten).unwrap();
    let o = f.run(&["logs", id]);
    assert_eq!(o.status.code(), Some(7));
    failure(o, "CORRUPT_REF");
}

#[cfg(unix)]
#[test]
fn snapshot_excludes_a_store_reached_through_an_ancestor_alias_before_and_after_creation() {
    let f = Fixture::new();
    let aliases = tempfile::tempdir().unwrap();
    std::os::unix::fs::symlink(f.0.path(), aliases.path().join("alias")).unwrap();
    let aliased_store = aliases.path().join("alias/new/nested/store");
    let snapshot = success(
        f.command()
            .env("CPYREF_STORE", &aliased_store)
            .args(["snapshot", "src"])
            .output()
            .unwrap(),
    );
    let id = run_reference(&snapshot);
    assert_eq!(
        success(
            f.command()
                .env("CPYREF_STORE", &aliased_store)
                .args(["snapshot", "check", id])
                .output()
                .unwrap()
        )["status"],
        "unchanged"
    );
    let all = success(
        f.command()
            .env("CPYREF_STORE", &aliased_store)
            .args(["snapshot", "."])
            .output()
            .unwrap(),
    );
    assert_eq!(all["file_count"], 2);
    assert_eq!(
        success(
            f.command()
                .env("CPYREF_STORE", &aliased_store)
                .args(["snapshot", "check", run_reference(&all)])
                .output()
                .unwrap()
        )["status"],
        "unchanged"
    );
}

#[cfg(unix)]
#[test]
fn run_timeout_stops_sleeping_processes_and_inherited_pipes() {
    for script in [
        "printf started; sleep 30",
        "printf started; sleep 30 & exit 0",
        "trap '' TERM; sleep 30",
    ] {
        let f = Fixture::new();
        let start = std::time::Instant::now();
        let o = f.run(&["run", "--timeout", "1", "--", "sh", "-c", script]);
        assert!(start.elapsed() < std::time::Duration::from_secs(6));
        assert_eq!(
            o.status.code(),
            Some(124),
            "{}",
            String::from_utf8_lossy(&o.stderr)
        );
        let v: Value = serde_json::from_slice(&o.stdout).unwrap();
        assert_eq!(v["termination_reason"], "timeout");
        assert_eq!(v["logs_complete"], false);
        assert!(v["analysis"]["counts"].is_null());
        assert!(f.json(&["logs", run_reference(&v), "--failed"])["analysis"]["counts"].is_null());
    }
}

#[test]
fn run_combined_log_cap_preserves_bounded_prefix_and_unknown_counts() {
    let f = Fixture::new();
    fs::write(f.source("large"), vec![b'x'; 32 * 1024]).unwrap();
    let id = f.copy("large", &[]);
    let o = f.child_run(&["--max-log-bytes", "100"], &["paste", &id]);
    assert_eq!(
        o.status.code(),
        Some(125),
        "{}",
        String::from_utf8_lossy(&o.stderr)
    );
    let v: Value = serde_json::from_slice(&o.stdout).unwrap();
    assert_eq!(v["termination_reason"], "log_limit");
    assert_eq!(v["logs_complete"], false);
    assert_eq!(
        v["stdout_bytes"].as_u64().unwrap() + v["stderr_bytes"].as_u64().unwrap(),
        100
    );
    assert!(v["analysis"]["counts"].is_null());
    let saved = f.json(&["logs", run_reference(&v), "--stream", "stdout"]);
    assert_eq!(saved["total_bytes"], 100);
}

#[test]
fn run_limits_config_environment_cli_precedence_and_preflight() {
    let f = Fixture::new();
    let config = f.source("limits.toml");
    fs::write(
        &config,
        "[run]\nmax_log_bytes = 2000\ntimeout_seconds = 2\nmin_free_bytes = 0\n",
    )
    .unwrap();
    let o = f
        .command()
        .env_remove("CPYREF_NO_CONFIG")
        .args([
            "--config",
            config.to_str().unwrap(),
            "run",
            "--max-log-bytes",
            "3000",
            "--",
            env!("CARGO_BIN_EXE_cpyref"),
            "--version",
        ])
        .env("CPYREF_RUN_MAX_LOG_BYTES", "2500")
        .env("CPYREF_RUN_TIMEOUT_SECONDS", "3")
        .output()
        .unwrap();
    let v = success(o);
    assert_eq!(v["limits"]["max_log_bytes"], 3000);
    assert_eq!(v["limits"]["timeout_seconds"], 3);
    assert_eq!(v["limits"]["min_free_bytes"], 0);
    let out = f.run(&[
        "run",
        "--min-free-bytes",
        "18446744073709551615",
        "--",
        "definitely-not-a-command",
    ]);
    failure(out, "DISK_SPACE");
    failure(
        f.run(&["run", "--timeout", "0", "--", "x"]),
        "INVALID_ARGUMENT",
    );
    failure(
        f.run(&["run", "--max-log-bytes", "0", "--", "x"]),
        "INVALID_ARGUMENT",
    );
    fs::write(&config, "[run]\ntimeout_seconds = 0\n").unwrap();
    failure(
        f.command()
            .env_remove("CPYREF_NO_CONFIG")
            .args(["--config", config.to_str().unwrap(), "context"])
            .output()
            .unwrap(),
        "CONFIG_ERROR",
    );
}

#[cfg(unix)]
#[test]
fn run_caps_stdout_and_stderr_together_even_with_a_complete_test_summary() {
    let f = Fixture::new();
    let script = "printf 'running 1 test\ntest result: ok. 1 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out;\n'; i=0; while [ $i -lt 1000 ]; do printf 'out'; printf 'err' >&2; i=$((i+1)); done";
    let o = f.run(&[
        "run",
        "--parser",
        "cargo",
        "--max-log-bytes",
        "512",
        "--",
        "sh",
        "-c",
        script,
    ]);
    assert_eq!(
        o.status.code(),
        Some(125),
        "{}",
        String::from_utf8_lossy(&o.stderr)
    );
    let v: Value = serde_json::from_slice(&o.stdout).unwrap();
    assert!(v["stdout_bytes"].as_u64().unwrap() + v["stderr_bytes"].as_u64().unwrap() <= 512);
    assert!(v["analysis"]["counts"].is_null());
    assert!(f.json(&["logs", run_reference(&v), "--failed"])["analysis"]["counts"].is_null());
}
