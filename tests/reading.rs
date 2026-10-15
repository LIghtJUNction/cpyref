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
    fn copy(&self, bytes: &[u8]) -> String {
        fs::write(self.path("input"), bytes).unwrap();
        let output = self.run(&["copy", "input"]);
        assert!(output.status.success());
        String::from_utf8(output.stdout).unwrap().trim().to_owned()
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
    fn pipe(&self, producer: &[&str], consumer: &[&str]) -> (Output, Output) {
        let mut first = self
            .command(producer)
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .unwrap();
        let second = self
            .command(consumer)
            .stdin(first.stdout.take().unwrap())
            .output()
            .unwrap();
        (first.wait_with_output().unwrap(), second)
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
fn bounded_pages_and_tail_keep_absolute_payload_positions() {
    let fixture = Fixture::new();
    let id = fixture.copy(&vec![b'x'; 9000]);
    let first = fixture.json(&["read", &id]);
    assert_eq!(first["bytes"], 4096);
    assert_eq!(first["content"].as_str().unwrap().len(), 4096);
    assert_eq!(first["total_bytes"], 9000);
    assert_eq!(first["next_offset"], 4096);
    assert_eq!(first["truncated"], true);
    assert_eq!(first["encoding"], "utf-8-lossy");
    assert_eq!(first["sha256"], fixture.json(&["info", &id])["sha256"]);
    assert!(first.get("selection_start").is_none());

    let second = fixture.json(&["read", &id, "--offset", "4096", "--max-bytes", "7"]);
    assert_eq!(second["offset"], 4096);
    assert_eq!(second["next_offset"], 4103);
    assert_eq!(second["bytes"], 7);
    assert_eq!(second["content"], "xxxxxxx");
    let tail = fixture.json(&["read", &id, "--tail", "--max-bytes", "7"]);
    assert_eq!(tail["offset"], 8993);
    assert_eq!(tail["next_offset"], 9000);
    assert_eq!(tail["truncated"], false);

    for offset in ["9000", "9001", "18446744073709551615"] {
        let eof = fixture.json(&["read", &id, "--offset", offset]);
        assert_eq!(eof["offset"], 9000);
        assert_eq!(eof["next_offset"], 9000);
        assert_eq!(eof["bytes"], 0);
        assert_eq!(eof["content"], "");
        assert_eq!(eof["truncated"], false);
    }
}

#[test]
fn lines_are_relative_to_saved_selection_and_keep_binary_and_crlf() {
    let fixture = Fixture::new();
    fs::write(fixture.path("input"), b"skip\r\nfirst\r\n\0\xff\r\nlast").unwrap();
    let copied = fixture.run(&["copy", "input", "--lines", "2:4"]);
    assert!(copied.status.success());
    let id = String::from_utf8(copied.stdout).unwrap();
    let id = id.trim();
    let bounded = fixture.json(&["read", id, "--lines", "2:3", "--max-bytes", "3"]);
    assert_eq!(bounded["offset"], 7);
    assert_eq!(bounded["selection_start"], 7);
    assert_eq!(bounded["selection_end"], 15);
    assert_eq!(bounded["next_offset"], 10);
    assert_eq!(bounded["content"], "\0\u{fffd}\r");
    assert_eq!(bounded["truncated"], true);
    let complete = fixture.json(&["read", id, "--lines", "2"]);
    assert_eq!(complete["content"], "\0\u{fffd}\r\n");
    assert_eq!(complete["selection_end"], 11);
    assert_eq!(complete["truncated"], false);
    let resumed = fixture.json(&["read", id, "--offset", "10", "--max-bytes", "5"]);
    assert_eq!(resumed["content"], "\nlast");
    assert_eq!(resumed["next_offset"], 15);
}

#[test]
fn line_ranges_reuse_next_offset_without_leaking_beyond_the_selection() {
    let fixture = Fixture::new();
    let id = fixture.copy(b"before\nabc\ndef\nAFTER\n");
    let mut offset = 0_u64;
    let mut collected = String::new();
    for (expected, next, truncated) in [("abc", 10, true), ("\nde", 13, true), ("f\n", 15, false)] {
        let page = fixture.json(&[
            "read",
            &id,
            "--lines",
            "2:3",
            "--offset",
            &offset.to_string(),
            "--max-bytes",
            "3",
        ]);
        assert_eq!(page["content"], expected);
        assert_eq!(page["offset"], offset.max(7));
        assert_eq!(page["next_offset"], next);
        assert_eq!(page["selection_start"], 7);
        assert_eq!(page["selection_end"], 15);
        assert_eq!(page["truncated"], truncated);
        collected.push_str(page["content"].as_str().unwrap());
        offset = page["next_offset"].as_u64().unwrap();
    }
    assert_eq!(collected, "abc\ndef\n");

    for offset in ["0", "1", "6"] {
        let first = fixture.json(&[
            "read",
            &id,
            "--lines",
            "2:3",
            "--offset",
            offset,
            "--max-bytes",
            "3",
        ]);
        assert_eq!(first["offset"], 7);
        assert_eq!(first["content"], "abc");
    }
    for offset in ["15", "16", "18446744073709551615"] {
        let eof = fixture.json(&["read", &id, "--lines", "2:3", "--offset", offset]);
        assert_eq!(eof["offset"], 15);
        assert_eq!(eof["next_offset"], 15);
        assert_eq!(eof["selection_start"], 7);
        assert_eq!(eof["selection_end"], 15);
        assert_eq!(eof["bytes"], 0);
        assert_eq!(eof["content"], "");
        assert_eq!(eof["truncated"], false);
    }
}

#[test]
fn line_boundaries_handle_final_newlines_blank_lines_and_stream_chunks() {
    let fixture = Fixture::new();
    let mut bytes = vec![b'a'; 65535];
    bytes.extend_from_slice(b"\n\r\nlast\n");
    let id = fixture.copy(&bytes);
    let blank = fixture.json(&["read", &id, "--lines", "2"]);
    assert_eq!(blank["offset"], 65536);
    assert_eq!(blank["content"], "\r\n");
    let last = fixture.json(&["read", &id, "--lines", "3"]);
    assert_eq!(last["content"], "last\n");
    let error = bad(fixture.run(&["read", &id, "--lines", "4"]), "NO_MATCH");
    assert_eq!(error["error"]["details"]["line_count"], 3);

    let newline = fixture.copy(b"\n");
    assert_eq!(
        fixture.json(&["read", &newline, "--lines", "1"])["content"],
        "\n"
    );
    bad(fixture.run(&["read", &newline, "--lines", "2"]), "NO_MATCH");
}

#[test]
fn saved_reads_survive_source_changes_and_escape_terminal_controls() {
    let fixture = Fixture::new();
    let id = fixture.copy(b"original\r\n\0\xff\x1b[31m");
    fs::write(fixture.path("input"), b"changed").unwrap();
    fs::remove_file(fixture.path("input")).unwrap();
    let output = fixture.run(&["read", &id]);
    assert!(!output.stdout.contains(&0x1b));
    assert!(!output.stdout.contains(&0));
    assert_eq!(good(output)["content"], "original\r\n\0\u{fffd}\u{1b}[31m");
    assert_eq!(fixture.json(&["read", &id, "--tail"])["offset"], 0);
}

#[test]
fn corruption_outside_requested_bytes_prevents_every_preview() {
    let fixture = Fixture::new();
    let id = fixture.copy(b"prefix\nuntouched\ncorrupt here");
    let context = fixture.json(&["context"]);
    let path = PathBuf::from(context["store_path"].as_str().unwrap()).join(format!("{id}.ref"));
    let mut stored = fs::read(&path).unwrap();
    *stored.last_mut().unwrap() ^= 1;
    fs::write(path, stored).unwrap();
    bad(
        fixture.run(&["read", &id, "--max-bytes", "1"]),
        "CORRUPT_REF",
    );
    bad(fixture.run(&["read", &id, "--lines", "1"]), "CORRUPT_REF");
}

#[test]
fn reading_stays_scoped_and_rejects_typed_records() {
    let fixture = Fixture::new();
    let id = fixture.copy(b"body");
    bad(
        fixture.run(&["--namespace", "other", "read", &id]),
        "REF_NOT_FOUND",
    );
    let snapshot = fixture.json(&["snapshot", "input"])["reference"]
        .as_str()
        .unwrap()
        .to_owned();
    bad(fixture.run(&["read", &snapshot]), "WRONG_KIND");
    let bundle = fixture.run(&["bundle", "create", &format!("body={id}")]);
    assert!(bundle.status.success());
    let bundle = String::from_utf8(bundle.stdout).unwrap();
    bad(fixture.run(&["read", bundle.trim()]), "WRONG_KIND");
}

#[test]
fn reference_pipes_accept_plain_and_success_json_but_never_switch_spaces() {
    let fixture = Fixture::new();
    fixture.copy(b"pipe body");
    for args in [vec!["copy", "input"], vec!["copy", "input", "--json"]] {
        let (producer, consumer) = fixture.pipe(&args, &["read", "-", "--max-bytes", "4"]);
        assert!(producer.status.success());
        assert_eq!(good(consumer)["content"], "pipe");
    }
    let copied = fixture.run(&["copy", "input", "--json"]);
    assert!(copied.status.success());
    bad(
        fixture.input(&["--namespace", "other", "read", "-"], &copied.stdout),
        "REF_CONTEXT",
    );
    bad(
        fixture.input(&["read", "-"], b"not a reference"),
        "REF_INPUT",
    );
}

#[test]
fn invalid_ranges_modes_and_caps_fail_without_creating_a_store() {
    let fixture = Fixture::new();
    for args in [
        vec!["read", "c0123456789ab", "--max-bytes", "0"],
        vec!["read", "c0123456789ab", "--max-bytes", "65537"],
        vec!["read", "c0123456789ab", "--tail", "--offset", "0"],
        vec!["read", "c0123456789ab", "--lines", "1", "--tail"],
    ] {
        bad(fixture.run(&args), "INVALID_ARGUMENT");
    }
    assert!(!fixture.path("store").exists());
    let id = fixture.copy(b"line\n");
    for range in ["0", "2:1", "bad", "1:2:3", "18446744073709551616"] {
        bad(
            fixture.run(&["read", &id, "--lines", range]),
            "INVALID_ARGUMENT",
        );
    }

    let store = cpyref::store::Store::new(fixture.path("absent"));
    for (offset, cap, lines, tail) in [
        (0, 0, None, false),
        (0, 65537, None, false),
        (1, 4096, None, true),
        (0, 4096, Some("1"), true),
    ] {
        assert_eq!(
            cpyref::inspect::read(&store, "c0123456789ab", offset, cap, lines, tail)
                .unwrap_err()
                .code,
            "INVALID_ARGUMENT"
        );
    }
    assert!(!fixture.path("absent").exists());
}
