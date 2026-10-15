use cpyref::{
    find::{find, Options},
    select::Span,
    store::{RecordKind, Store},
};
use serde_json::Value;
use std::{fs, io::Write, process::Command};
use tempfile::TempDir;

struct Fixture {
    dir: TempDir,
    store: Store,
}

impl Fixture {
    fn new() -> Self {
        let dir = tempfile::tempdir().unwrap();
        let store = Store::new(dir.path().join("store"));
        Self { dir, store }
    }

    fn copy(&self, bytes: &[u8]) -> String {
        self.store
            .put(
                bytes,
                "<stdin>".into(),
                Span {
                    byte_start: 0,
                    byte_end: bytes.len(),
                    line_start: 1,
                    line_end: 1,
                    selector: "all".into(),
                },
            )
            .unwrap()
            .id
    }

    fn cmd(&self, args: &[&str]) -> Command {
        let mut command = Command::new(env!("CARGO_BIN_EXE_cpyref"));
        for (key, _) in std::env::vars_os() {
            if key.to_str().is_some_and(|k| k.starts_with("CPYREF_")) {
                command.env_remove(key);
            }
        }
        command
            .current_dir(self.dir.path())
            .env("CPYREF_NO_CONFIG", "true")
            .env("CPYREF_STORE", self.dir.path().join("cli-store"))
            .args(args);
        command
    }
}

fn hits(report: &Value) -> &[Value] {
    report["matches"].as_array().unwrap()
}

#[test]
fn matches_paginate_by_count_with_saved_byte_and_line_positions() {
    let f = Fixture::new();
    let id = f.copy(b"one\nneedle\nneedle tail\nneedle");
    let options = Options {
        offset: 1,
        limit: 1,
        context_bytes: 0,
        ..Options::default()
    };
    let report = find(&f.store, &id, "needle", &options).unwrap();
    assert_eq!(report["total"], 3);
    assert_eq!(report["offset"], 1);
    assert_eq!(report["next_offset"], 2);
    assert_eq!(report["truncated"], true);
    assert_eq!(hits(&report).len(), 1);
    let hit = &hits(&report)[0];
    assert_eq!(hit["byte_start"], 11);
    assert_eq!(hit["byte_end"], 17);
    assert_eq!(hit["line_start"], 3);
    assert_eq!(hit["line_end"], 3);
    assert_eq!(hit["content"], "needle");
    let last = find(
        &f.store,
        &id,
        "needle",
        &Options {
            offset: 2,
            ..options
        },
    )
    .unwrap();
    assert_eq!(last["next_offset"], 3);
    assert_eq!(last["truncated"], false);
    let empty = find(
        &f.store,
        &id,
        "needle",
        &Options {
            offset: usize::MAX,
            ..options
        },
    )
    .unwrap();
    assert_eq!(empty["next_offset"], 3);
    assert!(hits(&empty).is_empty());
}

#[test]
fn total_excerpt_budget_keeps_all_hit_positions_after_exhaustion() {
    let f = Fixture::new();
    let id = f.copy(b"before needle after needle end");
    let report = find(
        &f.store,
        &id,
        "needle",
        &Options {
            max_bytes: 7,
            context_bytes: 80,
            ..Options::default()
        },
    )
    .unwrap();
    assert_eq!(report["total"], 2);
    assert_eq!(hits(&report).len(), 2);
    assert_eq!(report["content_bytes"], 7);
    assert_eq!(report["content_truncated"], true);
    assert_eq!(hits(&report)[0]["content"], "needle ");
    assert_eq!(hits(&report)[1]["byte_start"], 20);
    assert_eq!(hits(&report)[1]["content_start"], 20);
    assert_eq!(hits(&report)[1]["content_end"], 20);
    assert_eq!(hits(&report)[1]["content"], "");
    assert_eq!(hits(&report)[1]["context_truncated"], true);
    assert_eq!(report["truncated"], false);
    let small = find(
        &f.store,
        &id,
        "needle",
        &Options {
            max_bytes: 2,
            ..Options::default()
        },
    )
    .unwrap();
    assert_eq!(hits(&small)[0]["content"], "ne");
    assert_eq!(small["content_bytes"], 2);
}

#[test]
fn missing_queries_succeed_without_body_or_store_changes() {
    let f = Fixture::new();
    let bytes = b"private saved body";
    let id = f.copy(bytes);
    let report = find(&f.store, &id, "missing", &Options::default()).unwrap();
    assert_eq!(report["total"], 0);
    assert!(hits(&report).is_empty());
    assert_eq!(report["content_bytes"], 0);
    assert_eq!(report["content_truncated"], false);
    assert!(!report.to_string().contains("private saved body"));
    assert_eq!(f.store.list().unwrap().len(), 1);
    assert_eq!(f.store.get(&id).unwrap().1, bytes);
}

#[test]
fn literal_binary_control_and_multiline_matches_keep_exact_positions() {
    let f = Fixture::new();
    let bytes = b"\xff\0\x1b[31m\r\n.*\r\n.*\r\nend";
    let id = f.copy(bytes);
    let report = find(&f.store, &id, ".*\r\n", &Options::default()).unwrap();
    assert_eq!(report["total"], 2);
    assert_eq!(hits(&report)[0]["byte_start"], 9);
    assert_eq!(hits(&report)[0]["byte_end"], 13);
    assert_eq!(hits(&report)[0]["line_start"], 2);
    assert_eq!(hits(&report)[0]["line_end"], 2);
    assert_eq!(hits(&report)[1]["line_start"], 3);
    assert_eq!(hits(&report)[1]["line_end"], 3);
    let rendered = serde_json::to_vec(&report).unwrap();
    assert!(!rendered.contains(&0));
    assert!(!rendered.contains(&0x1b));
    assert!(hits(&report)[0]["content"]
        .as_str()
        .unwrap()
        .contains('\u{fffd}'));
    let multiline = find(&f.store, &id, ".*\r\n.*", &Options::default()).unwrap();
    assert_eq!(hits(&multiline)[0]["line_start"], 2);
    assert_eq!(hits(&multiline)[0]["line_end"], 3);
    assert_eq!(f.store.get(&id).unwrap().1, bytes);
}

#[test]
fn case_insensitive_unicode_matches_report_actual_byte_widths() {
    let f = Fixture::new();
    let id = f.copy("Kelvin kelvin KELVIN".as_bytes());
    let report = find(
        &f.store,
        &id,
        "kelvin",
        &Options {
            ignore_case: true,
            context_bytes: 0,
            ..Options::default()
        },
    )
    .unwrap();
    assert_eq!(report["total"], 3);
    assert_eq!(hits(&report)[0]["byte_start"], 0);
    assert_eq!(hits(&report)[0]["byte_end"], 8);
    assert_eq!(hits(&report)[0]["content"], "Kelvin");
    assert_eq!(
        find(&f.store, &id, "kelvin", &Options::default()).unwrap()["total"],
        1
    );
}

#[test]
fn literal_queries_use_non_overlapping_matches() {
    let f = Fixture::new();
    let id = f.copy(b"aaaaa");
    let report = find(&f.store, &id, "aa", &Options::default()).unwrap();
    assert_eq!(report["total"], 2);
    assert_eq!(report["match_mode"], "non_overlapping");
    assert_eq!(hits(&report)[0]["byte_start"], 0);
    assert_eq!(hits(&report)[1]["byte_start"], 2);
}

#[test]
fn corruption_outside_excerpt_is_rejected_even_without_any_matches() {
    let f = Fixture::new();
    let id = f.copy(b"needle far away from corrupt byte");
    let path = f.dir.path().join("store").join(format!("{id}.ref"));
    let mut data = fs::read(&path).unwrap();
    *data.last_mut().unwrap() ^= 1;
    fs::write(path, data).unwrap();
    for query in ["needle", "missing"] {
        let error = find(
            &f.store,
            &id,
            query,
            &Options {
                max_bytes: 1,
                ..Options::default()
            },
        )
        .unwrap_err();
        assert_eq!(error.code, "CORRUPT_REF");
    }
}

#[test]
fn library_validates_all_limits_and_rejects_typed_records() {
    let f = Fixture::new();
    let id = f.copy(b"needle");
    for options in [
        Options {
            limit: 0,
            ..Options::default()
        },
        Options {
            limit: 1001,
            ..Options::default()
        },
        Options {
            max_bytes: 0,
            ..Options::default()
        },
        Options {
            max_bytes: 65537,
            ..Options::default()
        },
        Options {
            context_bytes: 4097,
            ..Options::default()
        },
    ] {
        assert_eq!(
            find(&f.store, &id, "needle", &options).unwrap_err().code,
            "INVALID_ARGUMENT"
        );
    }
    for query in ["".to_owned(), "x".repeat(4097), "界".repeat(1366)] {
        assert_eq!(
            find(&f.store, &id, &query, &Options::default())
                .unwrap_err()
                .code,
            "INVALID_ARGUMENT"
        );
    }
    let mut file = f.store.staging_file().unwrap();
    file.write_all(b"{}").unwrap();
    let typed = f
        .store
        .put_record(file.as_file_mut(), RecordKind::Snapshot, vec![], 0)
        .unwrap();
    assert_eq!(
        find(&f.store, &typed.id, "needle", &Options::default())
            .unwrap_err()
            .code,
        "WRONG_KIND"
    );
}

#[test]
fn cli_outputs_bounded_json_and_honors_namespace_isolation() {
    let f = Fixture::new();
    fs::write(f.dir.path().join("input"), b"one needle\nsecond needle\n").unwrap();
    let copied = f.cmd(&["copy", "input"]).output().unwrap();
    assert!(copied.status.success());
    let id = String::from_utf8(copied.stdout).unwrap();
    let output = f
        .cmd(&[
            "find",
            id.trim(),
            "needle",
            "--limit",
            "1",
            "--max-bytes",
            "6",
            "--context-bytes",
            "0",
        ])
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(output.stderr.is_empty());
    let report: Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(report["total"], 2);
    assert_eq!(report["content_bytes"], 6);
    assert_eq!(hits(&report)[0]["content"], "needle");
    let absent = f
        .cmd(&["--namespace", "other", "find", id.trim(), "needle"])
        .output()
        .unwrap();
    assert!(!absent.status.success());
    assert!(absent.stdout.is_empty());
    let report: Value = serde_json::from_slice(&absent.stderr).unwrap();
    assert_eq!(report["error"]["code"], "REF_NOT_FOUND");
}
