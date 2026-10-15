use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::{fs, io::Write, path::PathBuf, process::{Command, Output, Stdio}};
use tempfile::TempDir;

struct Fixture { dir: TempDir }
impl Fixture {
    fn new() -> Self {
        let f = Self { dir: tempfile::tempdir().unwrap() };
        fs::create_dir(f.path(".git")).unwrap();
        fs::write(f.path("source"), b"new\x00\xff\r\n").unwrap();
        fs::write(f.path("target"), b"first\r\nold\r\nlast\r\n").unwrap();
        f
    }
    fn path(&self, name: &str) -> PathBuf { self.dir.path().join(name) }
    fn cmd(&self, args: &[&str]) -> Command {
        let mut c = Command::new(env!("CARGO_BIN_EXE_cpyref"));
        for (key, _) in std::env::vars_os() {
            if key.to_str().is_some_and(|k| k.starts_with("CPYREF_")) { c.env_remove(key); }
        }
        c.current_dir(self.dir.path()).env("CPYREF_NO_CONFIG", "true")
            .env("CPYREF_STORE", self.path("store")).args(args);
        c
    }
    fn run(&self, args: &[&str]) -> Output { self.cmd(args).output().unwrap() }
    fn input(&self, args: &[&str], bytes: &[u8]) -> Output {
        let mut child = self.cmd(args).stdin(Stdio::piped())
            .stdout(Stdio::piped()).stderr(Stdio::piped()).spawn().unwrap();
        let _ = child.stdin.take().unwrap().write_all(bytes);
        child.wait_with_output().unwrap()
    }
    fn pipe(&self, producer: &[&str], consumer: &[&str]) -> (Output, Output) {
        let mut first = self.cmd(producer).stdout(Stdio::piped()).stderr(Stdio::piped()).spawn().unwrap();
        let second = self.cmd(consumer).stdin(first.stdout.take().unwrap()).output().unwrap();
        (first.wait_with_output().unwrap(), second)
    }
    fn copy(&self) -> String {
        let o = self.run(&["copy", "source"]);
        assert!(o.status.success());
        String::from_utf8(o.stdout).unwrap().trim().to_owned()
    }
    fn plan(&self, id: &str) -> Value {
        good(self.run(&["insert", id, "target", "--at-end", "--plan"]))
    }
    fn target(&self) -> Vec<u8> { fs::read(self.path("target")).unwrap() }
    fn count(&self) -> u64 { good(self.run(&["stats"]))["count"].as_u64().unwrap() }
    fn record(&self, id: &str) -> PathBuf {
        let ctx = good(self.run(&["context"]));
        PathBuf::from(ctx["store_path"].as_str().unwrap()).join(format!("{id}.ref"))
    }
}
fn good(o: Output) -> Value {
    assert!(o.status.success(), "{}", String::from_utf8_lossy(&o.stderr));
    assert!(o.stderr.is_empty());
    serde_json::from_slice(&o.stdout).unwrap()
}
fn bad(o: Output, code: &str) {
    assert!(!o.status.success());
    assert!(o.stdout.is_empty());
    let v: Value = serde_json::from_slice(&o.stderr).unwrap();
    assert_eq!(v["error"]["code"], code, "{v}");
}
fn id(plan: &Value) -> &str { plan["plan"].as_str().unwrap() }

#[test]
fn file_copy_pipes_plain_and_json_refs_without_expanding_binary_payloads() {
    let f = Fixture::new();
    for (flags, dest) in [(vec!["copy", "source"], "out"), (vec!["copy", "source", "--json"], "json-out")] {
        let (a, b) = f.pipe(&flags, &["paste", "-", "--output", dest]);
        assert!(a.status.success());
        assert!(b.status.success(), "{}", String::from_utf8_lossy(&b.stderr));
        assert!(b.stdout.is_empty());
        assert_eq!(fs::read(f.path(dest)).unwrap(), fs::read(f.path("source")).unwrap());
    }
}

#[test]
fn piped_copy_plans_insert_then_applies_json_without_manual_hashes() {
    let f = Fixture::new();
    let before = f.target();
    let (a, b) = f.pipe(&["copy", "source"], &["insert", "-", "target", "--after-line", "1", "--plan"]);
    assert!(a.status.success());
    let p = good(b);
    assert_eq!(p["applied"], false);
    assert_eq!(p["status"], "planned");
    assert_eq!(p["next_command"], format!("cpyref apply {}", id(&p)));
    assert_eq!(f.target(), before);
    let applied = good(f.input(&["apply", "-"], &serde_json::to_vec(&p).unwrap()));
    assert_eq!(applied["applied"], true);
    assert_eq!(f.target(), b"first\r\nnew\x00\xff\r\nold\r\nlast\r\n");
    bad(f.run(&["apply", id(&p)]), "TARGET_CHANGED");
}

#[test]
fn replace_plan_can_be_rechecked_without_writing() {
    let f = Fixture::new();
    let r = f.copy();
    let p = good(f.run(&["replace", &r, "target", "--lines", "2", "--plan"]));
    let before = f.target();
    assert_eq!(good(f.run(&["apply", id(&p), "--dry-run"]))["applied"], false);
    assert_eq!(f.target(), before);
    assert_eq!(good(f.run(&["apply", id(&p)]))["applied"], true);
    assert_eq!(f.target(), b"first\r\nnew\x00\xff\r\nlast\r\n");
}

#[test]
fn failed_copy_cannot_create_or_apply_a_plan() {
    let f = Fixture::new();
    let before = f.target();
    let (a, b) = f.pipe(&["copy", "missing"], &["insert", "-", "target", "--at-end", "--plan"]);
    assert!(!a.status.success());
    bad(b, "REF_INPUT");
    assert_eq!(f.target(), before);
    assert_eq!(f.count(), 0);
}

#[test]
fn malformed_empty_large_and_multiple_reference_inputs_fail_closed() {
    let f = Fixture::new();
    let r = f.copy();
    for bytes in [Vec::new(), b" \r\n".to_vec(), b"payload".to_vec(), vec![255],
        format!("{r}\n{r}\n").into_bytes(), vec![b' '; 65537],
        format!(r#"{{"ok":false,"reference":{{"id":"{r}"}}}}"#).into_bytes()] {
        bad(f.input(&["paste", "-", "--output", "out"], &bytes), "REF_INPUT");
        assert!(!f.path("out").exists());
    }
}

#[test]
fn plain_reference_accepts_crlf_but_never_an_embedded_id() {
    let f = Fixture::new();
    let r = f.copy();
    let o = f.input(&["paste", "-"], format!(" {r}\r\n").as_bytes());
    assert!(o.status.success());
    assert_eq!(o.stdout, fs::read(f.path("source")).unwrap());
    bad(f.input(&["paste", "-"], format!("result: {r}\n").as_bytes()), "REF_INPUT");
}

#[test]
fn piped_json_does_not_redirect_the_context_or_accept_failed_results() {
    let f = Fixture::new();
    let mut v = good(f.run(&["copy", "source", "--json"]));
    v["context"]["namespace"] = json!("other");
    bad(f.input(&["paste", "-"], &serde_json::to_vec(&v).unwrap()), "REF_CONTEXT");
    v["ok"] = json!(false);
    bad(f.input(&["paste", "-"], &serde_json::to_vec(&v).unwrap()), "REF_INPUT");
}

#[test]
fn plan_stays_in_its_namespace_and_protects_its_source_reference() {
    let f = Fixture::new();
    let r = f.copy();
    let p = f.plan(&r);
    let before = f.target();
    bad(f.run(&["--namespace", "other", "apply", id(&p)]), "REF_NOT_FOUND");
    bad(f.input(&["--namespace", "other", "apply", "-"], &serde_json::to_vec(&p).unwrap()), "REF_CONTEXT");
    bad(f.run(&["remove", &r]), "REF_IN_USE");
    assert_eq!(f.target(), before);
    assert!(f.run(&["remove", id(&p)]).status.success());
    assert!(f.run(&["remove", &r]).status.success());
}

#[test]
fn a_stale_plan_never_refreshes_its_target_hash() {
    let f = Fixture::new();
    let r = f.copy();
    let p = f.plan(&r);
    fs::write(f.path("target"), b"changed by another editor").unwrap();
    bad(f.run(&["apply", id(&p)]), "TARGET_CHANGED");
    bad(f.run(&["apply", id(&p), "--dry-run"]), "TARGET_CHANGED");
    assert_eq!(f.target(), b"changed by another editor");
}

#[test]
fn plan_uses_saved_bytes_not_later_source_file_changes() {
    let f = Fixture::new();
    let r = f.copy();
    let p = f.plan(&r);
    fs::remove_file(f.path("source")).unwrap();
    good(f.run(&["apply", id(&p)]));
    assert!(f.target().ends_with(b"new\x00\xff\r\n"));
}

#[test]
fn corrupted_plan_and_bytes_disguised_as_plan_are_rejected() {
    let f = Fixture::new();
    let r = f.copy();
    bad(f.run(&["apply", &r]), "WRONG_KIND");
    let p = f.plan(&r);
    let before = f.target();
    let path = f.record(id(&p));
    let mut bytes = fs::read(&path).unwrap();
    *bytes.last_mut().unwrap() ^= 1;
    fs::write(path, bytes).unwrap();
    bad(f.run(&["apply", id(&p)]), "CORRUPT_REF");
    assert_eq!(f.target(), before);
}

#[test]
fn source_reference_hash_is_bound_to_the_plan() {
    let f = Fixture::new();
    let r = f.copy();
    let p = f.plan(&r);
    let before = f.target();
    let path = f.record(&r);
    let bytes = fs::read(&path).unwrap();
    let n = u32::from_le_bytes(bytes[8..12].try_into().unwrap()) as usize;
    let mut header: Value = serde_json::from_slice(&bytes[12..12+n]).unwrap();
    let mut payload = bytes[12+n..].to_vec();
    payload[0] = b'X';
    header["sha256"] = json!(format!("{:x}", Sha256::digest(&payload)));
    let header = serde_json::to_vec(&header).unwrap();
    let mut altered = bytes[..8].to_vec();
    altered.extend_from_slice(&(header.len() as u32).to_le_bytes());
    altered.extend(header);
    altered.extend(payload);
    fs::write(path, altered).unwrap();
    bad(f.run(&["apply", id(&p)]), "REF_CHANGED");
    assert_eq!(f.target(), before);
}

#[test]
fn plans_respect_capacity_and_dry_run_stays_read_only() {
    let f = Fixture::new();
    let r = f.copy();
    let before = f.target();
    assert_eq!(f.count(), 1);
    good(f.run(&["insert", &r, "target", "--at-end", "--dry-run"]));
    assert_eq!(f.count(), 1);
    bad(f.run(&["--max-entries", "1", "insert", &r, "target", "--at-end", "--plan"]), "ENTRY_LIMIT");
    assert_eq!(f.count(), 1);
    assert_eq!(f.target(), before);
}

#[test]
fn existing_hash_route_and_required_precondition_are_preserved() {
    let f = Fixture::new();
    let r = f.copy();
    bad(f.run(&["insert", &r, "target", "--at-end"]), "PRECONDITION_REQUIRED");
    bad(f.run(&["insert", &r, "target", "--at-end", "--plan", "--dry-run"]), "INVALID_ARGUMENT");
    let preview = good(f.run(&["insert", &r, "target", "--at-end", "--dry-run"]));
    good(f.run(&["insert", &r, "target", "--at-end", "--expect-sha256", preview["before_sha256"].as_str().unwrap()]));
    assert_eq!(f.count(), 1);
}

#[test]
fn ambiguous_or_mixed_selectors_do_not_create_plans() {
    let f = Fixture::new();
    let r = f.copy();
    fs::write(f.path("target"), b"old old").unwrap();
    bad(f.run(&["replace", &r, "target", "--text", "old", "--plan"]), "AMBIGUOUS");
    bad(f.run(&["copy", "target", "--lines", "1", "--from", "old", "--to", "old"]), "INVALID_ARGUMENT");
    assert_eq!(f.target(), b"old old");
    assert_eq!(f.count(), 1);
}

#[test]
fn concurrent_applications_of_a_changing_plan_do_not_double_insert() {
    let f = Fixture::new();
    let r = f.copy();
    let p = f.plan(&r);
    let mut a = f.cmd(&["apply", id(&p)]);
    let mut b = f.cmd(&["apply", id(&p)]);
    let a = a.stdout(Stdio::piped()).stderr(Stdio::piped()).spawn().unwrap();
    let b = b.stdout(Stdio::piped()).stderr(Stdio::piped()).spawn().unwrap();
    let a = a.wait_with_output().unwrap();
    let b = b.wait_with_output().unwrap();
    assert_ne!(a.status.success(), b.status.success());
    bad(if a.status.success() { b } else { a }, "TARGET_CHANGED");
    assert_eq!(f.target(), b"first\r\nold\r\nlast\r\nnew\x00\xff\r\n");
}

#[test]
fn handoff_of_a_plan_is_read_only_and_paste_refuses_it() {
    let f = Fixture::new();
    let r = f.copy();
    let p = f.plan(&r);
    let before = f.target();
    let ticket = f.run(&["handoff", id(&p), "--format", "ref"]);
    assert!(ticket.status.success());
    let ticket = String::from_utf8(ticket.stdout).unwrap();
    assert_eq!(good(f.run(&["receive", ticket.trim()]))["applied"], false);
    bad(f.run(&["paste", id(&p)]), "WRONG_KIND");
    assert_eq!(f.target(), before);
}

#[test]
fn plan_target_is_absolute_and_does_not_move_when_cwd_changes() {
    let f = Fixture::new();
    let r = f.copy();
    let p = f.plan(&r);
    fs::create_dir(f.path("sub")).unwrap();
    fs::write(f.path("sub/target"), b"leave this alone").unwrap();
    let o = f.cmd(&["apply", id(&p)]).current_dir(f.path("sub")).output().unwrap();
    good(o);
    assert_eq!(fs::read(f.path("sub/target")).unwrap(), b"leave this alone");
    assert!(f.target().ends_with(b"new\x00\xff\r\n"));
}

#[test]
fn pipe_info_and_remove_keep_the_original_single_id_contract() {
    let f = Fixture::new();
    let r = f.copy();
    assert_eq!(good(f.input(&["info", "-"], r.as_bytes()))["id"], r);
    assert!(f.input(&["remove", "-"], r.as_bytes()).status.success());
    assert_eq!(f.count(), 0);
}

#[test]
fn copy_plan_apply_can_run_as_an_actual_three_process_pipeline() {
    let f = Fixture::new();
    let mut copy = f.cmd(&["copy", "source"]).stdout(Stdio::piped()).stderr(Stdio::piped()).spawn().unwrap();
    let mut plan = f.cmd(&["insert", "-", "target", "--at-end", "--plan"])
        .stdin(copy.stdout.take().unwrap()).stdout(Stdio::piped()).stderr(Stdio::piped()).spawn().unwrap();
    let result = f.cmd(&["apply", "-"]).stdin(plan.stdout.take().unwrap()).output().unwrap();
    assert!(copy.wait_with_output().unwrap().status.success());
    assert!(plan.wait_with_output().unwrap().status.success());
    assert_eq!(good(result)["applied"], true);
    assert_eq!(f.target(), b"first\r\nold\r\nlast\r\nnew\x00\xff\r\n");
}
