use serde_json::Value;
use std::{
    fs,
    path::PathBuf,
    process::{Command, Output},
};
use tempfile::TempDir;

struct Fixture(TempDir);
impl Fixture {
    fn new() -> Self {
        let f = Self(tempfile::tempdir().unwrap());
        fs::create_dir(f.path(".git")).unwrap();
        f
    }
    fn path(&self, name: &str) -> PathBuf {
        self.0.path().join(name)
    }
    fn run(&self, args: &[&str]) -> Output {
        Command::new(env!("CARGO_BIN_EXE_cpyref"))
            .current_dir(self.0.path())
            .env_remove("CPYREF_CONFIG")
            .env_remove("CPYREF_PROJECT")
            .env_remove("CPYREF_SCOPE")
            .env_remove("CPYREF_NAMESPACE")
            .env_remove("CPYREF_MAX_ENTRIES")
            .env("CPYREF_NO_CONFIG", "true")
            .env("CPYREF_STORE", self.path("store"))
            .args(args)
            .output()
            .unwrap()
    }
    fn text(&self, args: &[&str]) -> String {
        let o = self.run(args);
        assert!(o.status.success(), "{}", String::from_utf8_lossy(&o.stderr));
        String::from_utf8(o.stdout).unwrap().trim_end().to_owned()
    }
    fn json(&self, args: &[&str]) -> Value {
        serde_json::from_str(&self.text(args)).unwrap()
    }
    fn copy(&self, name: &str, bytes: &[u8]) -> String {
        fs::write(self.path(name), bytes).unwrap();
        self.text(&["copy", name])
    }
}
fn failure(o: Output, code: &str) {
    assert!(!o.status.success());
    assert!(o.stdout.is_empty());
    let v: Value = serde_json::from_slice(&o.stderr).unwrap();
    assert_eq!(v["error"]["code"], code);
}

#[test]
fn full_handoff_restores_from_another_project_without_inheriting_context() {
    let f = Fixture::new();
    let receiver = Fixture::new();
    let bytes = b"exact\r\n\0\xff";
    fs::write(f.path("input"), bytes).unwrap();
    let id = f.text(&["--namespace", "sender", "copy", "input"]);
    let report = f.json(&["--namespace", "sender", "handoff", &id]);
    assert_eq!(report["sha256"].as_str().unwrap().len(), 64);
    let token = report["token"].as_str().unwrap();
    let o = Command::new(env!("CARGO_BIN_EXE_cpyref"))
        .current_dir(receiver.0.path())
        .env_remove("CPYREF_NO_CONFIG")
        .env("CPYREF_CONFIG", "/missing/config")
        .env("CPYREF_PROJECT", "/missing/project")
        .env("CPYREF_NAMESPACE", "wrong")
        .env("CPYREF_STORE", receiver.path("unrelated"))
        .args(["receive", token])
        .output()
        .unwrap();
    assert!(o.status.success(), "{}", String::from_utf8_lossy(&o.stderr));
    assert_eq!(o.stdout, bytes);
    assert!(!receiver.path("unrelated").exists());
    assert_eq!(
        f.text(&["--namespace", "sender", "handoff", &id, "--format", "ref"]),
        token
    );
}

#[test]
fn receive_checks_hash_and_output_protection_before_writing() {
    let f = Fixture::new();
    let id = f.copy("input", b"saved");
    let token = f.text(&["handoff", &id, "--format", "ref"]);
    let mut value: Value = serde_json::from_str(token.strip_prefix("cpyref:v1:").unwrap()).unwrap();
    value["sha256"] = Value::String("0".repeat(64));
    let bad = format!("cpyref:v1:{value}");
    failure(
        f.run(&["receive", &bad, "--output", "missing"]),
        "HASH_MISMATCH",
    );
    assert!(!f.path("missing").exists());
    fs::write(f.path("existing"), b"protected").unwrap();
    failure(
        f.run(&["receive", &token, "--output", "existing"]),
        "OUTPUT_EXISTS",
    );
    assert_eq!(fs::read(f.path("existing")).unwrap(), b"protected");
    f.text(&["receive", &token, "--output", "existing", "--force"]);
    assert_eq!(fs::read(f.path("existing")).unwrap(), b"saved");
    failure(f.run(&["receive", "cpyref:v2:{}"]), "INVALID_ARGUMENT");
}

#[test]
fn bundle_indexes_metadata_and_reads_only_one_bounded_item() {
    let f = Fixture::new();
    let patch = f.copy("patch", b"patch\r\n\0\xff");
    let notes = f.copy("notes", &vec![b'x'; 9000]);
    let bundle = f.text(&[
        "bundle",
        "create",
        &format!("patch={patch}"),
        &format!("notes={notes}"),
    ]);
    let index = f.json(&["bundle", "show", &bundle, "--limit", "1"]);
    assert_eq!(index["item_count"], 2);
    assert_eq!(index["omitted"], 1);
    assert!(!index.to_string().contains("xxxx"));
    let read = f.json(&[
        "bundle",
        "read",
        &bundle,
        "notes",
        "--max-bytes",
        "10",
        "--offset",
        "3",
    ]);
    assert_eq!(read["bytes"], 10);
    assert_eq!(read["content"], "xxxxxxxxxx");
    assert_eq!(read["next_offset"], 13);
    assert_eq!(read["truncated"], true);
    f.json(&["bundle", "read", &bundle, "patch", "--output", "restored"]);
    assert_eq!(fs::read(f.path("restored")).unwrap(), b"patch\r\n\0\xff");
    failure(
        f.run(&["bundle", "read", &bundle, "missing"]),
        "ITEM_NOT_FOUND",
    );
    failure(f.run(&["bundle", "show", &patch]), "WRONG_KIND");
    assert_eq!(
        f.json(&["search", "bundle", "--kind", "bundle", "--json"])["total"],
        1
    );
}

#[test]
fn bundle_handoff_selects_one_item_and_protects_dependencies() {
    let f = Fixture::new();
    let patch = f.copy("patch", b"patch");
    let notes = f.copy("notes", b"notes");
    let bundle = f.text(&[
        "bundle",
        "create",
        &format!("patch={patch}"),
        &format!("notes={notes}"),
    ]);
    let token = f.text(&["handoff", &bundle, "--format", "ref"]);
    assert_eq!(f.json(&["receive", &token])["item_count"], 2);
    assert_eq!(f.text(&["receive", &token, "--item", "notes"]), "notes");
    failure(f.run(&["remove", &patch]), "REF_IN_USE");
    failure(
        f.run(&[
            "bundle",
            "create",
            &format!("a={patch}"),
            &format!("a={notes}"),
        ]),
        "INVALID_ARGUMENT",
    );
    failure(
        f.run(&["bundle", "create", &format!("../bad={patch}")]),
        "INVALID_ARGUMENT",
    );
    failure(
        f.run(&[
            "--namespace",
            "other",
            "bundle",
            "create",
            &format!("patch={patch}"),
        ]),
        "REF_NOT_FOUND",
    );
    f.text(&["remove", &bundle]);
    f.text(&["remove", &patch]);
}

#[test]
fn typed_receipts_and_snapshots_stay_bounded_and_never_execute_on_receive() {
    let f = Fixture::new();
    fs::write(f.path("input"), b"input").unwrap();
    let snapshot = f.json(&["snapshot", "input"])["reference"]
        .as_str()
        .unwrap()
        .to_owned();
    let run = f.json(&["run", "--", env!("CARGO_BIN_EXE_cpyref"), "--version"])["reference"]
        .as_str()
        .unwrap()
        .to_owned();
    let bundle = f.text(&[
        "bundle",
        "create",
        &format!("source={snapshot}"),
        &format!("tests={run}"),
    ]);
    let receipt = f.json(&["bundle", "read", &bundle, "tests"]);
    assert_eq!(receipt["reference"], run);
    assert!(!receipt.to_string().contains("content"));
    let token = f.text(&["handoff", &bundle, "--format", "ref"]);
    assert_eq!(
        f.json(&["receive", &token, "--item", "source"])["file_count"],
        1
    );
    failure(
        f.run(&["receive", &token, "--item", "tests", "--output", "raw"]),
        "INVALID_ARGUMENT",
    );
    assert!(!f.path("raw").exists());
}

#[test]
fn directory_and_explicit_shared_handoffs_keep_the_original_space() {
    for scope in ["directory", "shared", "global"] {
        let f = Fixture::new();
        let receiver = Fixture::new();
        fs::write(f.path("input"), b"exact").unwrap();
        let id = f.text(&["--scope", scope, "--namespace", "default", "copy", "input"]);
        let token = f.text(&[
            "--scope",
            scope,
            "--namespace",
            "default",
            "handoff",
            &id,
            "--format",
            "ref",
        ]);
        let o = Command::new(env!("CARGO_BIN_EXE_cpyref"))
            .current_dir(receiver.0.path())
            .env_remove("CPYREF_CONFIG")
            .env_remove("CPYREF_NO_CONFIG")
            .args(["receive", &token])
            .output()
            .unwrap();
        assert!(o.status.success(), "{}", String::from_utf8_lossy(&o.stderr));
        assert_eq!(o.stdout, b"exact");
    }
}
