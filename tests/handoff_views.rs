use serde_json::Value;
use std::{
    fs,
    path::{Path, PathBuf},
    process::{Command, Output},
};
use tempfile::TempDir;

struct Fixture(TempDir);

fn clear_context(command: &mut Command) {
    for (key, _) in std::env::vars_os() {
        if key.to_str().is_some_and(|key| key.starts_with("CPYREF_")) {
            command.env_remove(key);
        }
    }
}

impl Fixture {
    fn new() -> Self {
        let fixture = Self(
            tempfile::Builder::new()
                .prefix("cpyref sender's ")
                .tempdir()
                .unwrap(),
        );
        fs::create_dir(fixture.path(".git")).unwrap();
        fixture
    }

    fn path(&self, name: &str) -> PathBuf {
        self.0.path().join(name)
    }

    fn write(&self, name: &str, bytes: &[u8]) {
        fs::write(self.path(name), bytes).unwrap();
    }

    fn command_at(&self, executable: &Path) -> Command {
        let mut command = Command::new(executable);
        clear_context(&mut command);
        command
            .current_dir(self.0.path())
            .env("CPYREF_NO_CONFIG", "true")
            .env("CPYREF_STORE", self.path("store"));
        command
    }

    fn command(&self) -> Command {
        self.command_at(Path::new(env!("CARGO_BIN_EXE_cpyref")))
    }

    fn run(&self, args: &[&str]) -> Output {
        self.command().args(args).output().unwrap()
    }

    fn json(&self, args: &[&str]) -> Value {
        good(self.run(args))
    }

    fn copy(&self, bytes: &[u8]) -> String {
        self.write("input", bytes);
        let output = self.run(&["copy", "input"]);
        assert!(output.status.success(), "{:?}", output);
        assert!(output.stderr.is_empty());
        String::from_utf8(output.stdout).unwrap().trim().to_owned()
    }

    fn ticket(&self, id: &str) -> String {
        self.json(&["handoff", id])["token"]
            .as_str()
            .unwrap()
            .to_owned()
    }

    fn package(&self, notes: &[u8], attachment: &[u8]) -> Value {
        self.write("notes", notes);
        self.write("attachment", attachment);
        self.json(&[
            "handoff",
            "--input",
            "notes",
            "--attach",
            "patch=attachment",
            "--format",
            "json",
        ])
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
fn help_puts_one_step_handoff_before_the_copy_recipe_and_explains_cpy() {
    let fixture = Fixture::new();
    let output = fixture.run(&["--help"]);
    assert!(output.status.success());
    assert!(output.stderr.is_empty());
    let help = String::from_utf8(output.stdout).unwrap();
    let handoff = help
        .find("cpyref handoff --input HANDOFF.md --attach patch=change.patch")
        .unwrap();
    let copy = help.find("cpyref copy source.txt").unwrap();
    assert!(handoff < copy);
    assert!(help.contains("cpyref handoff --input -"));
    assert!(help.contains("'cpy' means cpyref"));
    assert!(help.contains("alias cpy=cpyref"));
    assert!(!fixture.path("store").exists());
}

#[test]
fn new_handoff_defaults_to_a_forwardable_prompt_with_one_token_and_no_notes() {
    let fixture = Fixture::new();
    let secret = "SAVED_PRIVATE_NOTES_MUST_NOT_BE_EXPANDED";
    fixture.write("notes", secret.as_bytes());
    fixture.write("attachment", b"SAVED_PRIVATE_PATCH_MUST_NOT_BE_EXPANDED");
    let output = fixture.run(&[
        "handoff",
        "--input",
        "notes",
        "--attach",
        "patch=attachment",
    ]);
    assert!(output.status.success(), "{:?}", output);
    assert!(output.stderr.is_empty());
    let prompt = String::from_utf8(output.stdout).unwrap();
    assert_eq!(prompt.matches("cpyref:v1:").count(), 1);
    assert!(!prompt.contains(secret));
    assert!(!prompt.contains("SAVED_PRIVATE_PATCH"));
    for fact in [
        "同机",
        "原存储",
        "历史快照",
        "当前文件",
        "执行授权",
        "--output",
    ] {
        assert!(prompt.contains(fact), "Missing {fact}: {prompt}");
    }
    assert!(prompt.find("--list").unwrap() < prompt.find("--read").unwrap());
    assert!(prompt.contains("2048"));
    assert!(prompt.contains(if cfg!(windows) {
        "```powershell"
    } else {
        "```sh"
    }));
    assert_eq!(fixture.json(&["stats"])["count"], 3);
}

#[test]
fn existing_reference_handoff_keeps_json_by_default_and_agent_is_explicit() {
    let fixture = Fixture::new();
    let id = fixture.copy(b"old byte reference\r\n");
    let report = fixture.json(&["handoff", &id]);
    assert_eq!(report["reference"], id);
    assert_eq!(report["kind"], "bytes");
    assert!(report["token"].as_str().unwrap().starts_with("cpyref:v1:"));
    assert!(report.get("prompt").is_none());
    assert!(report.get("created").is_none());
    let agent = fixture.run(&["handoff", &id, "--format", "agent"]);
    assert!(agent.status.success(), "{:?}", agent);
    let prompt = String::from_utf8(agent.stdout).unwrap();
    assert_eq!(prompt.matches("cpyref:v1:").count(), 1);
    assert!(!prompt.contains("old byte reference"));
    let restored = fixture.run(&["receive", report["token"].as_str().unwrap()]);
    assert!(restored.status.success());
    assert_eq!(restored.stdout, b"old byte reference\r\n");
    assert_eq!(fixture.json(&["stats"])["count"], 1);
}

#[test]
fn received_package_directory_is_bounded_and_pages_without_payloads() {
    let fixture = Fixture::new();
    fixture.write("notes", b"PRIVATE_NOTES_PAYLOAD");
    fixture.write("attachment", b"PRIVATE_ATTACHMENT_PAYLOAD");
    let mut args = vec!["handoff".to_owned(), "--input".into(), "notes".into()];
    for index in 0..23 {
        args.extend(["--attach".into(), format!("item_{index:02}=attachment")]);
    }
    args.extend(["--format".into(), "json".into()]);
    let report = good(fixture.command().args(args).output().unwrap());
    let token = report["token"].as_str().unwrap();
    let first = fixture.json(&["receive", token, "--list"]);
    assert_eq!(first["kind"], "bundle");
    assert_eq!(first["item_count"], 24);
    assert_eq!(first["offset"], 0);
    assert_eq!(first["items"].as_array().unwrap().len(), 20);
    assert_eq!(first["omitted"], 4);
    assert!(!first.to_string().contains("PRIVATE_"));
    let second = fixture.json(&["receive", token, "--list", "--offset", "20", "--limit", "2"]);
    assert_eq!(second["items"].as_array().unwrap().len(), 2);
    assert_eq!(second["offset"], 20);
    assert_eq!(second["omitted"], 2);
    assert_ne!(first["items"][0]["name"], second["items"][0]["name"]);
    let all = fixture.json(&["receive", token, "--list", "--limit", "100"]);
    assert_eq!(all["items"].as_array().unwrap().len(), 24);
    assert_eq!(all["omitted"], 0);
    let empty = fixture.json(&[
        "receive",
        token,
        "--list",
        "--offset",
        "18446744073709551615",
        "--limit",
        "1",
    ]);
    assert!(empty["items"].as_array().unwrap().is_empty());
    assert_eq!(empty["omitted"], 0);
}

#[test]
fn received_byte_directory_returns_only_metadata() {
    let fixture = Fixture::new();
    let payload = b"PRIVATE_BYTES_NOT_A_DIRECTORY_ENTRY";
    let id = fixture.copy(payload);
    let token = fixture.ticket(&id);
    let report = fixture.json(&["receive", &token, "--list"]);
    assert_eq!(report["reference"], id);
    assert_eq!(report["kind"], "bytes");
    assert_eq!(report["bytes"], payload.len());
    assert_eq!(report["historical_snapshot"], true);
    assert_eq!(report["transport"], "same_host");
    assert_eq!(report["sha256"], fixture.json(&["info", &id])["sha256"]);
    assert!(report.get("content").is_none());
    assert!(report.get("items").is_none());
    assert!(!report.to_string().contains("PRIVATE_BYTES"));
}

#[test]
fn received_notes_read_in_pages_from_the_saved_snapshot() {
    let fixture = Fixture::new();
    let notes = b"header\r\nabc\0\xffremaining";
    let package = fixture.package(notes, b"patch");
    let token = package["token"].as_str().unwrap();
    fixture.write("notes", b"changed after the handoff");
    let first = fixture.json(&[
        "receive",
        token,
        "--item",
        "notes",
        "--read",
        "--max-bytes",
        "6",
    ]);
    assert_eq!(first["content"], "header");
    assert_eq!(first["bytes"], 6);
    assert_eq!(first["next_offset"], 6);
    assert_eq!(first["total_bytes"], notes.len());
    assert_eq!(first["truncated"], true);
    let second = fixture.json(&[
        "receive",
        token,
        "--item",
        "notes",
        "--read",
        "--offset",
        "6",
        "--max-bytes",
        "4",
    ]);
    assert_eq!(second["content"], "\r\nab");
    assert_eq!(second["offset"], first["next_offset"]);
    assert_eq!(second["next_offset"], 10);
    assert_eq!(second["reference"], first["reference"]);
    let binary = fixture.json(&[
        "receive",
        token,
        "--item",
        "notes",
        "--read",
        "--offset",
        "10",
        "--max-bytes",
        "3",
    ]);
    assert_eq!(binary["content"], "c\0\u{fffd}");
    assert_eq!(binary["encoding"], "utf-8-lossy");
    let eof = fixture.json(&[
        "receive",
        token,
        "--item",
        "notes",
        "--read",
        "--offset",
        "18446744073709551615",
    ]);
    assert_eq!(eof["content"], "");
    assert_eq!(eof["next_offset"], notes.len());
    assert_eq!(eof["truncated"], false);
}

#[test]
fn received_byte_preview_defaults_to_4096_and_can_continue() {
    let fixture = Fixture::new();
    let id = fixture.copy(&vec![b'x'; 5000]);
    let token = fixture.ticket(&id);
    let first = fixture.json(&["receive", &token, "--read"]);
    assert_eq!(first["bytes"], 4096);
    assert_eq!(first["content"].as_str().unwrap().len(), 4096);
    assert_eq!(first["next_offset"], 4096);
    assert_eq!(first["truncated"], true);
    let next = fixture.json(&[
        "receive",
        &token,
        "--read",
        "--offset",
        "4096",
        "--max-bytes",
        "7",
    ]);
    assert_eq!(next["content"], "xxxxxxx");
    assert_eq!(next["next_offset"], 4103);
    let complete = fixture.json(&["receive", &token, "--read", "--max-bytes", "65536"]);
    assert_eq!(complete["bytes"], 5000);
    assert_eq!(complete["truncated"], false);
}

#[test]
fn receiving_exact_bytes_and_item_exports_preserves_output_protection() {
    let fixture = Fixture::new();
    let bytes = b"\0\xffline\r\nend";
    let id = fixture.copy(bytes);
    let token = fixture.ticket(&id);
    let raw = fixture.run(&["receive", &token]);
    assert!(raw.status.success());
    assert_eq!(raw.stdout, bytes);
    let package = fixture.package(b"notes", bytes);
    let token = package["token"].as_str().unwrap();
    fixture.write("attachment", b"new source bytes");
    let raw_item = fixture.run(&["receive", token, "--item", "patch"]);
    assert!(raw_item.status.success());
    assert_eq!(raw_item.stdout, bytes);
    let exported = fixture.run(&["receive", token, "--item", "patch", "--output", "restored"]);
    assert!(exported.status.success());
    assert!(exported.stdout.is_empty());
    assert_eq!(fs::read(fixture.path("restored")).unwrap(), bytes);
    fixture.write("restored", b"keep this file");
    bad(
        fixture.run(&["receive", token, "--item", "patch", "--output", "restored"]),
        "OUTPUT_EXISTS",
    );
    assert_eq!(
        fs::read(fixture.path("restored")).unwrap(),
        b"keep this file"
    );
    let replaced = fixture.run(&[
        "receive", token, "--item", "patch", "--output", "restored", "--force",
    ]);
    assert!(replaced.status.success());
    assert_eq!(fs::read(fixture.path("restored")).unwrap(), bytes);
}

#[test]
fn typed_references_and_typed_items_cannot_be_read_as_bytes() {
    let fixture = Fixture::new();
    let package = fixture.package(b"notes", b"patch");
    bad(
        fixture.run(&["receive", package["token"].as_str().unwrap(), "--read"]),
        "WRONG_KIND",
    );
    let snapshot = fixture.json(&["snapshot", "notes"]);
    let snapshot_id = snapshot["reference"].as_str().unwrap();
    let token = fixture.ticket(snapshot_id);
    bad(fixture.run(&["receive", &token, "--read"]), "WRONG_KIND");
    let item = format!("source={snapshot_id}");
    let output = fixture.run(&["bundle", "create", &item]);
    assert!(output.status.success());
    let id = String::from_utf8(output.stdout).unwrap();
    let token = fixture.ticket(id.trim());
    bad(
        fixture.run(&["receive", &token, "--item", "source", "--read"]),
        "WRONG_KIND",
    );
}

#[test]
fn conflicting_modes_and_out_of_range_receive_caps_fail_before_writing() {
    let fixture = Fixture::new();
    fixture.write("notes", b"must not be saved");
    let cases: &[&[&str]] = &[
        &["receive", "invalid", "--list", "--read"],
        &["receive", "invalid", "--list", "--item", "notes"],
        &["receive", "invalid", "--list", "--output", "output"],
        &["receive", "invalid", "--read", "--output", "output"],
        &["receive", "invalid", "--limit", "1"],
        &["receive", "invalid", "--max-bytes", "1"],
        &["receive", "invalid", "--offset", "1"],
        &["receive", "invalid", "--list", "--limit", "0"],
        &["receive", "invalid", "--list", "--limit", "101"],
        &["receive", "invalid", "--read", "--max-bytes", "0"],
        &["receive", "invalid", "--read", "--max-bytes", "65537"],
        &["receive", "invalid", "--force"],
        &["handoff", "c0123456789ab", "--input", "notes"],
        &["handoff", "c0123456789ab", "--max-bytes", "10"],
        &["handoff", "--attach", "patch=notes"],
        &["handoff", "--input", "notes", "--format", "unsupported"],
    ];
    for args in cases {
        let error = bad(fixture.run(args), "INVALID_ARGUMENT");
        assert!(error["error"]["details"]
            .get("created_references")
            .is_none());
        assert!(!fixture.path("store").exists(), "{args:?}");
        assert!(!fixture.path("output").exists(), "{args:?}");
    }
}

#[test]
fn store_path_is_the_exact_selected_json_path_and_does_not_create_storage() {
    let fixture = Fixture::new();
    let base = fixture.path("store's 空间");
    let base_text = base.to_str().unwrap();
    let common = [
        "--store",
        base_text,
        "--scope",
        "shared",
        "--namespace",
        "review",
    ];
    let report = good(
        fixture
            .command()
            .args(common)
            .arg("context")
            .output()
            .unwrap(),
    );
    let selected = report["store_path"].as_str().unwrap();
    assert_ne!(Path::new(selected), base);
    let output = fixture
        .command()
        .args(common)
        .args(["context", "--store-path"])
        .output()
        .unwrap();
    assert!(output.status.success(), "{:?}", output);
    assert!(output.stderr.is_empty());
    assert_eq!(output.stdout, format!("{selected}\n").as_bytes());
    assert!(!base.exists());
    assert!(!Path::new(selected).exists());
    assert!(!fixture.path("store").exists());
}

#[cfg(unix)]
#[test]
fn store_path_refuses_a_lossy_native_path_without_creating_it() {
    use std::{ffi::OsString, os::unix::ffi::OsStringExt};
    let fixture = Fixture::new();
    let base = fixture
        .0
        .path()
        .join(OsString::from_vec(b"store-\xff".to_vec()));
    bad(
        fixture
            .command()
            .arg("--store")
            .arg(&base)
            .args(["context", "--store-path"])
            .output()
            .unwrap(),
        "PATH_ENCODING",
    );
    assert!(!base.exists());
}

#[cfg(unix)]
#[test]
fn generated_agent_commands_survive_hostile_environment_paths_and_other_cwd() {
    let fixture = Fixture::new();
    let binary = Path::new(env!("CARGO_BIN_EXE_cpyref"));
    // Keep the quoted executable on the binary's filesystem without writing a
    // new executable inode that can transiently fail to spawn with ETXTBSY.
    let bin_dir = tempfile::Builder::new()
        .prefix("cpyref agent's bin ")
        .tempdir_in(binary.parent().unwrap())
        .unwrap();
    let executable = fs::canonicalize(bin_dir.path())
        .unwrap()
        .join("cpyref's executable");
    fs::hard_link(binary, &executable).unwrap();
    let mut notes = vec![b'x'; 5000];
    notes[..12].copy_from_slice(b"preview only");
    let tail = b"NEVER_EXPAND_TAIL!";
    notes[4096..4096 + tail.len()].copy_from_slice(tail);
    fixture.write("notes", &notes);
    fixture.write("attachment", b"patch bytes");
    let output = fixture
        .command_at(&executable)
        .args([
            "handoff",
            "--input",
            "notes",
            "--attach",
            "patch=attachment",
        ])
        .output()
        .unwrap();
    assert!(output.status.success(), "{:?}", output);
    let prompt = String::from_utf8(output.stdout).unwrap();
    let block = prompt
        .split_once("```sh\n")
        .unwrap()
        .1
        .split_once("\n```")
        .unwrap()
        .0;
    let quoted_executable = format!("'{}'", executable.to_str().unwrap().replace('\'', "'\\''"));
    assert!(block.contains(&quoted_executable), "{block}");
    let hostile = [
        ("CPYREF_CONFIG", "missing'config"),
        ("CPYREF_PROJECT", "missing project"),
        ("CPYREF_SCOPE", "invalid scope"),
        ("CPYREF_MAX_ENTRIES", "not a number"),
        ("CPYREF_NO_CONFIG", "not a boolean"),
        ("CPYREF_STORE", "wrong store"),
        ("CPYREF_NAMESPACE", "wrong/namespace"),
    ];
    let receiver = fixture.path("receiver's cwd");
    let empty_path = fixture.path("empty PATH");
    fs::create_dir(&receiver).unwrap();
    fs::create_dir(&empty_path).unwrap();
    let receiver = fs::canonicalize(receiver).unwrap();
    let script = format!(
        "set -eu\n{block}\nprintf '\\nparent-vars:%s|%s|%s|%s|%s|%s|%s\\n' \"$CPYREF_CONFIG\" \"$CPYREF_PROJECT\" \"$CPYREF_SCOPE\" \"$CPYREF_MAX_ENTRIES\" \"$CPYREF_NO_CONFIG\" \"$CPYREF_STORE\" \"$CPYREF_NAMESPACE\"\nprintf 'parent-cwd:%s\\n' \"$PWD\"\n"
    );
    let mut command = Command::new("/bin/sh");
    clear_context(&mut command);
    let output = command
        .current_dir(&receiver)
        .env("PATH", empty_path)
        .envs(hostile)
        .args(["-c", &script])
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(output.stderr.is_empty());
    let stdout = String::from_utf8(output.stdout).unwrap();
    let reports: Vec<Value> = stdout
        .lines()
        .filter(|line| line.starts_with('{'))
        .map(|line| serde_json::from_str(line).unwrap())
        .collect();
    assert_eq!(reports.len(), 2, "{stdout}");
    assert_eq!(reports[0]["kind"], "bundle");
    assert_eq!(reports[0]["item_count"], 2);
    assert_eq!(reports[1]["bytes"], 2048);
    assert_eq!(reports[1]["next_offset"], 2048);
    assert_eq!(reports[1]["truncated"], true);
    assert!(reports[1]["content"]
        .as_str()
        .unwrap()
        .starts_with("preview only"));
    assert!(!stdout.contains("NEVER_EXPAND_TAIL"));
    let values = hostile.map(|(_, value)| value).join("|");
    assert!(stdout.contains(&format!("parent-vars:{values}\n")));
    assert!(stdout.contains(&format!("parent-cwd:{}\n", receiver.display())));
}

#[cfg(target_os = "linux")]
#[test]
fn failed_handoff_stdout_reports_the_persisted_package_and_unsafe_retry() {
    let fixture = Fixture::new();
    fixture.write("notes", b"saved even when stdout fails");
    fixture.write("attachment", b"saved patch");
    let full = fs::OpenOptions::new()
        .write(true)
        .open("/dev/full")
        .unwrap();
    let output = fixture
        .command()
        .args([
            "handoff",
            "--input",
            "notes",
            "--attach",
            "patch=attachment",
        ])
        .stdout(full)
        .output()
        .unwrap();
    let failure = bad(output, "IO_ERROR");
    let details = &failure["error"]["details"];
    assert_eq!(details["stage"], "write_handoff");
    assert_eq!(details["retry_safe"], false);
    assert_eq!(details["side_effects"], true);
    let created = details["created_references"].as_array().unwrap();
    let package = created
        .iter()
        .find(|item| item["name"] == "package")
        .unwrap();
    let id = package["id"].as_str().unwrap();
    assert_eq!(fixture.json(&["info", id])["kind"], "bundle");
    assert_eq!(fixture.json(&["bundle", "show", id])["item_count"], 2);
    assert_eq!(
        fixture.json(&["bundle", "read", id, "notes"])["content"],
        "saved even when stdout fails"
    );
    assert_eq!(fixture.json(&["stats"])["count"], 3);
}
