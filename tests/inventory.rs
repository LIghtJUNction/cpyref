use serde_json::Value;
use std::{
    fs,
    process::{Command, Output, Stdio},
};
use tempfile::TempDir;

struct Fixture(TempDir);
impl Fixture {
    fn new() -> Self {
        let f = Self(tempfile::tempdir().unwrap());
        fs::create_dir(f.0.path().join(".git")).unwrap();
        fs::write(
            f.0.path().join("input"),
            "BEGIN\r\nprivate payload\r\nEND\r\n",
        )
        .unwrap();
        f
    }
    fn command(&self) -> Command {
        let mut c = Command::new(env!("CARGO_BIN_EXE_cpyref"));
        c.current_dir(self.0.path());
        for key in [
            "CPYREF_SCOPE",
            "CPYREF_NAMESPACE",
            "CPYREF_PROJECT",
            "CPYREF_CONFIG",
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
        success_json(self.run(args))
    }
    fn copy(&self, args: &[&str]) -> String {
        let o = self
            .command()
            .args(["copy", "input"])
            .args(args)
            .output()
            .unwrap();
        assert!(o.status.success(), "{}", String::from_utf8_lossy(&o.stderr));
        String::from_utf8(o.stdout).unwrap().trim().to_owned()
    }
}
fn success_json(o: Output) -> Value {
    assert!(o.status.success(), "{}", String::from_utf8_lossy(&o.stderr));
    assert!(o.stderr.is_empty());
    serde_json::from_slice(&o.stdout).unwrap()
}
fn failure(o: Output, code: &str) -> Value {
    assert!(!o.status.success());
    assert!(o.stdout.is_empty());
    let v: Value = serde_json::from_slice(&o.stderr).unwrap();
    assert_eq!(v["error"]["code"], code);
    v
}

#[test]
fn read_only_inventory_and_preview_create_no_store() {
    let f = Fixture::new();
    assert_eq!(f.json(&["list", "--json"])["total"], 0);
    assert_eq!(f.json(&["stats"])["count"], 0);
    assert_eq!(f.json(&["clean", "--all"])["count"], 0);
    let v = f.json(&[
        "copy",
        "input",
        "--from",
        "BEGIN",
        "--to",
        "END",
        "--exclude-end",
        "--dry-run",
    ]);
    assert_eq!(v["match_count"], 1);
    assert_eq!(v["selected"]["line_end"], 2);
    assert_eq!(v["bytes"], 24);
    assert!(v.get("content").is_none());
    assert!(!f.0.path().join("store").exists());
}

#[test]
fn preview_content_is_explicit_bounded_and_escaped() {
    let f = Fixture::new();
    let v = f.json(&[
        "copy",
        "input",
        "--dry-run",
        "--show-content",
        "--preview-bytes",
        "8",
    ]);
    assert_eq!(v["content"], "BEGIN\r\np");
    assert_eq!(v["content_bytes"], 8);
    assert_eq!(v["content_truncated"], true);
    failure(
        f.run(&["copy", "input", "--show-content"]),
        "INVALID_ARGUMENT",
    );
    failure(
        f.run(&[
            "copy",
            "input",
            "--dry-run",
            "--show-content",
            "--preview-bytes",
            "4097",
        ]),
        "INVALID_ARGUMENT",
    );
    assert!(!f.0.path().join("store").exists());
}

#[test]
fn independent_boundaries_preserve_original_bytes() {
    let f = Fixture::new();
    for (flags, expected) in [
        (vec![], b"BEGIN\r\nprivate payload\r\nEND".as_slice()),
        (
            vec!["--exclude-start"],
            b"\r\nprivate payload\r\nEND".as_slice(),
        ),
        (
            vec!["--exclude-end"],
            b"BEGIN\r\nprivate payload\r\n".as_slice(),
        ),
        (
            vec!["--exclude-start", "--exclude-end"],
            b"\r\nprivate payload\r\n".as_slice(),
        ),
        (
            vec!["--exclude-markers"],
            b"\r\nprivate payload\r\n".as_slice(),
        ),
    ] {
        let mut args = vec!["--from", "BEGIN", "--to", "END"];
        args.extend(flags);
        let id = f.copy(&args);
        assert_eq!(f.run(&["paste", &id]).stdout, expected);
    }
    failure(
        f.run(&[
            "copy",
            "input",
            "--from",
            "BEGIN",
            "--to",
            "END",
            "--exclude-markers",
            "--exclude-start",
        ]),
        "INVALID_ARGUMENT",
    );
}

#[test]
fn ambiguity_stays_an_error_with_bounded_locations() {
    let f = Fixture::new();
    fs::write(f.0.path().join("input"), "secret\n".repeat(25)).unwrap();
    let e = failure(
        f.run(&["copy", "input", "--text", "secret", "--dry-run"]),
        "AMBIGUOUS",
    );
    assert_eq!(e["error"]["details"]["match_count"], 25);
    assert_eq!(
        e["error"]["details"]["candidates"]
            .as_array()
            .unwrap()
            .len(),
        20
    );
    assert_eq!(e["error"]["details"]["truncated"], true);
    assert!(!e.to_string().contains("secret"));
    let v = f.json(&[
        "copy",
        "input",
        "--text",
        "secret",
        "--dry-run",
        "--occurrence",
        "23",
    ]);
    assert_eq!(v["match_count"], 25);
    assert_eq!(v["selected"]["line_start"], 23);
    assert!(!f.0.path().join("store").exists());
}

#[test]
fn notes_filters_pagination_and_old_format() {
    let f = Fixture::new();
    let old = f.copy(&[]);
    let annotated = f.copy(&["--note", "修改前\n\u{1b}[31m"]);
    let v = f.json(&["list", "--json", "--source", "input", "--note", "修改前"]);
    assert_eq!(v["total"], 1);
    assert_eq!(v["references"][0]["id"], annotated);
    assert_eq!(v["references"][0]["note"], "修改前\n\u{1b}[31m");
    assert!(!v.to_string().contains("private payload"));
    let output = f.run(&["list"]);
    assert!(output.status.success());
    assert_eq!(output.stdout.iter().filter(|&&b| b == b'\n').count(), 3);
    assert!(!output.stdout.contains(&0x1b));
    assert!(f.json(&["info", &old]).get("note").is_none());
    let page = f.json(&["list", "--json", "--limit", "1", "--offset", "1"]);
    assert_eq!(page["references"].as_array().unwrap().len(), 1);
    assert_eq!(page["total"], 2);
    assert_eq!(
        f.json(&["list", "--json", "--offset", "999"])["references"],
        serde_json::json!([])
    );
    assert_eq!(
        f.json(&["--namespace", "other", "list", "--json"])["total"],
        0
    );
    failure(
        f.run(&["copy", "input", "--note", &"x".repeat(4097)]),
        "INVALID_ARGUMENT",
    );
}

#[test]
fn stats_measure_payload_and_record_size_without_expanding_content() {
    let f = Fixture::new();
    f.copy(&[]);
    f.copy(&["--text", "private"]);
    let v = f.json(&["stats"]);
    assert_eq!(v["count"], 2);
    assert_eq!(v["payload_bytes"], 36);
    assert!(v["stored_bytes"].as_u64().unwrap() > 36);
    assert_eq!(v["age_buckets"]["under_1_day"], 2);
    assert_eq!(v["largest"]["bytes"], 29);
    assert!(!v.to_string().contains("private payload"));
}

#[test]
fn cleanup_requires_preview_token_and_refuses_changed_set() {
    let f = Fixture::new();
    let keep = f.copy(&["--note", "keep"]);
    let delete = f.copy(&["--note", "delete"]);
    failure(f.run(&["clean"]), "INVALID_ARGUMENT");
    failure(f.run(&["clean", "--all", "--apply"]), "INVALID_ARGUMENT");
    let p = f.json(&["clean", "--note", "delete"]);
    assert_eq!(p["count"], 1);
    assert_eq!(p["applied"], false);
    assert!(f.run(&["paste", &delete]).status.success());
    let changed = f.copy(&["--note", "delete"]);
    failure(
        f.run(&[
            "clean",
            "--note",
            "delete",
            "--apply",
            "--confirm",
            p["confirm"].as_str().unwrap(),
        ]),
        "PLAN_CHANGED",
    );
    assert!(f.run(&["paste", &delete]).status.success());
    let p = f.json(&["clean", &delete, &changed]);
    let result = f.json(&[
        "clean",
        &delete,
        &changed,
        "--apply",
        "--confirm",
        p["confirm"].as_str().unwrap(),
    ]);
    assert_eq!(result["count"], 2);
    assert!(f.run(&["paste", &keep]).status.success());
    failure(f.run(&["paste", &delete]), "REF_NOT_FOUND");
    assert_eq!(f.json(&["stats"])["count"], 1);
}

#[test]
fn cleanup_age_and_newest_protection_and_scope_binding() {
    let f = Fixture::new();
    f.copy(&[]);
    f.copy(&[]);
    assert_eq!(f.json(&["clean", "--older-than", "30"])["count"], 0);
    assert_eq!(f.json(&["clean", "--keep-latest", "1"])["count"], 1);
    assert_eq!(
        f.json(&["clean", "--all", "--keep-latest", "10"])["count"],
        0
    );
    let p = f.json(&["clean", "--all"]);
    failure(
        f.run(&[
            "--namespace",
            "other",
            "clean",
            "--all",
            "--apply",
            "--confirm",
            p["confirm"].as_str().unwrap(),
        ]),
        "PLAN_CHANGED",
    );
    assert_eq!(f.json(&["stats"])["count"], 2);
}

#[test]
fn concurrent_copies_cannot_exceed_entry_limit_and_cleanup_frees_slot() {
    let f = Fixture::new();
    let children: Vec<_> = (0..12)
        .map(|_| {
            f.command()
                .args(["--max-entries", "3", "copy", "input"])
                .stdout(Stdio::piped())
                .stderr(Stdio::piped())
                .spawn()
                .unwrap()
        })
        .collect();
    let mut count = 0;
    for child in children {
        let o = child.wait_with_output().unwrap();
        if o.status.success() {
            count += 1;
        } else {
            failure(o, "ENTRY_LIMIT");
        }
    }
    assert_eq!(count, 3);
    assert_eq!(f.json(&["stats"])["count"], 3);
    let p = f.json(&["clean", "--all"]);
    f.json(&[
        "clean",
        "--all",
        "--apply",
        "--confirm",
        p["confirm"].as_str().unwrap(),
    ]);
    f.copy(&["--max-entries", "3"]);
    assert_eq!(f.json(&["stats"])["count"], 1);
}

#[test]
fn corrupt_inventory_fails_closed_but_explicit_remove_remains_available() {
    let f = Fixture::new();
    let id = f.copy(&[]);
    let c = f.json(&["context"]);
    fs::write(
        std::path::Path::new(c["store_path"].as_str().unwrap()).join(format!("{id}.ref")),
        "broken",
    )
    .unwrap();
    failure(f.run(&["list"]), "CORRUPT_REF");
    failure(f.run(&["clean", "--all"]), "CORRUPT_REF");
    failure(
        f.run(&["--max-entries", "1", "copy", "input"]),
        "ENTRY_LIMIT",
    );
    assert!(f.run(&["remove", &id]).status.success());
}

#[test]
fn config_limit_is_enforced_with_environment_and_cli_overrides() {
    let f = Fixture::new();
    fs::write(f.0.path().join(".cpyref.toml"), "max_entries = 1\n").unwrap();
    let copy = || {
        let mut c = f.command();
        c.env_remove("CPYREF_NO_CONFIG");
        c.args(["copy", "input"]);
        c
    };
    assert!(copy().output().unwrap().status.success());
    failure(copy().output().unwrap(), "ENTRY_LIMIT");
    assert!(copy()
        .env("CPYREF_MAX_ENTRIES", "2")
        .output()
        .unwrap()
        .status
        .success());
    failure(
        copy().env("CPYREF_MAX_ENTRIES", "2").output().unwrap(),
        "ENTRY_LIMIT",
    );
    assert!(copy()
        .env("CPYREF_MAX_ENTRIES", "2")
        .args(["--max-entries", "3"])
        .output()
        .unwrap()
        .status
        .success());
    failure(copy().output().unwrap(), "ENTRY_LIMIT");
    assert_eq!(f.json(&["stats"])["count"], 3);
    assert!(copy()
        .args(["--max-entries", "0"])
        .output()
        .unwrap()
        .status
        .success());
    failure(f.run(&["clean", "--note", ""]), "INVALID_ARGUMENT");
}

#[test]
fn configured_cleanup_is_only_used_when_invoked_and_explicit_selections_override_it() {
    let f = Fixture::new();
    for _ in 0..3 {
        f.copy(&[]);
    }
    fs::write(
        f.0.path().join(".cpyref.toml"),
        "[cleanup]\nolder_than_days = 30\nkeep_latest = 2\n",
    )
    .unwrap();
    let clean = |args: &[&str]| {
        success_json(
            f.command()
                .env_remove("CPYREF_NO_CONFIG")
                .arg("clean")
                .args(args)
                .output()
                .unwrap(),
        )
    };
    assert_eq!(clean(&[])["count"], 0);
    assert_eq!(clean(&["--all"])["count"], 3);
    assert_eq!(clean(&["--all", "--keep-latest", "1"])["count"], 2);
    fs::write(
        f.0.path().join(".cpyref.toml"),
        "[cleanup]\nkeep_latest = 2\n",
    )
    .unwrap();
    let p = clean(&[]);
    assert_eq!(p["count"], 1);
    assert_eq!(f.json(&["stats"])["count"], 3);
    assert_eq!(clean(&["--keep-latest", "1"])["count"], 2);
    let id = p["references"][0]["id"].as_str().unwrap();
    assert_eq!(clean(&[id])["count"], 1);
    assert_eq!(
        clean(&["--apply", "--confirm", p["confirm"].as_str().unwrap()])["count"],
        1
    );
    assert_eq!(f.json(&["stats"])["count"], 2);
}

#[test]
fn search_finds_old_unannotated_content_and_metadata_without_expanding_it() {
    let f = Fixture::new();
    let id = f.copy(&[]);
    let note = f.copy(&["--note", "修改前的函数"]);
    let v = f.json(&["search", "private payload", "--json"]);
    assert_eq!(v["total"], 2);
    assert!(v["references"]
        .as_array()
        .unwrap()
        .iter()
        .any(|r| r["id"] == id));
    assert!(!v.to_string().contains("private payload"));
    assert_eq!(
        f.json(&["search", "private payload", "--metadata-only", "--json"])["total"],
        0
    );
    let v = f.json(&["search", "修改前", "--json"]);
    assert_eq!(v["references"][0]["id"], note);
    assert_eq!(
        v["references"][0]["matched_in"],
        serde_json::json!(["note"])
    );
    assert_eq!(
        f.json(&["search", "input", "--metadata-only", "--json"])["total"],
        2
    );
    assert_eq!(f.json(&["search", &id[1..7], "--json"])["total"], 1);
    assert_eq!(
        f.json(&["search", "PRIVATE PAYLOAD", "--ignore-case", "--json"])["total"],
        2
    );
    assert_eq!(f.json(&["search", "PRIVATE PAYLOAD", "--json"])["total"], 0);
    let v = f.json(&[
        "search", "private", "--json", "--limit", "1", "--offset", "1",
    ]);
    assert_eq!(v["total"], 2);
    assert_eq!(v["references"].as_array().unwrap().len(), 1);
    assert_eq!(
        f.json(&["--namespace", "other", "search", "private", "--json"])["total"],
        0
    );
    assert_eq!(
        f.json(&["search", "private", "--kind", "snapshot", "--json"])["total"],
        0
    );
}

#[test]
fn search_handles_binary_and_chunk_boundary_matches_and_rejects_corruption() {
    let f = Fixture::new();
    let mut bytes = vec![0xff; 65534];
    bytes.extend_from_slice("Kelvin needle\0".as_bytes());
    fs::write(f.0.path().join("input"), bytes).unwrap();
    let o = f.run(&["copy", "input"]);
    assert!(o.status.success());
    let id = String::from_utf8(o.stdout).unwrap().trim().to_owned();
    assert_eq!(f.json(&["search", "Kelvin needle", "--json"])["total"], 1);
    assert_eq!(
        f.json(&["search", "kelvin needle", "--ignore-case", "--json"])["total"],
        1
    );
    let context = f.json(&["context"]);
    let path =
        std::path::Path::new(context["store_path"].as_str().unwrap()).join(format!("{id}.ref"));
    let mut bytes = fs::read(&path).unwrap();
    *bytes.last_mut().unwrap() ^= 1;
    fs::write(path, bytes).unwrap();
    failure(f.run(&["search", "needle", "--json"]), "CORRUPT_REF");
    assert_eq!(
        f.json(&["search", "input", "--metadata-only", "--json"])["total"],
        1
    );
}

#[test]
fn search_is_read_only_and_empty_or_oversized_queries_are_rejected() {
    let f = Fixture::new();
    assert_eq!(f.json(&["search", "missing", "--json"])["total"], 0);
    failure(f.run(&["search", ""]), "INVALID_ARGUMENT");
    failure(f.run(&["search", &"x".repeat(4097)]), "INVALID_ARGUMENT");
    assert!(!f.0.path().join("store").exists());
}
