use serde_json::Value;
use std::{collections::HashSet, fs, path::{Path, PathBuf}, process::{Command, Output, Stdio}};
use tempfile::TempDir;

struct Fixture {
    dir: TempDir,
    a: PathBuf,
    b: PathBuf,
    base: PathBuf,
}

fn success(output: Output) -> Vec<u8> {
    assert!(output.status.success(), "{}", String::from_utf8_lossy(&output.stderr));
    assert!(output.stderr.is_empty());
    output.stdout
}

fn reference(output: Output) -> String {
    let text = String::from_utf8(success(output)).unwrap();
    assert_eq!(text.len(), 14);
    assert!(text.ends_with('\n'));
    text.trim().to_owned()
}

fn failure(output: Output, exit: i32, code: &str) -> Value {
    assert_eq!(output.status.code(), Some(exit), "{}", String::from_utf8_lossy(&output.stderr));
    assert!(output.stdout.is_empty());
    let error: Value = serde_json::from_slice(&output.stderr).unwrap();
    assert_eq!(error["error"]["code"], code);
    error
}

impl Fixture {
    fn new() -> Self {
        let dir = tempfile::tempdir().unwrap();
        let a = dir.path().join("a");
        let b = dir.path().join("b");
        for (path, payload) in [(&a, b"PROJECT_A_SECRET"), (&b, b"PROJECT_B_SECRET")] {
            fs::create_dir_all(path.join(".git")).unwrap();
            fs::create_dir_all(path.join("src")).unwrap();
            fs::write(path.join("input"), payload).unwrap();
        }
        let base = dir.path().join("store");
        Self { dir, a, b, base }
    }

    fn command(&self, cwd: &Path) -> Command {
        let mut cmd = Command::new(env!("CARGO_BIN_EXE_cpyref"));
        cmd.current_dir(cwd);
        for key in ["CPYREF_STORE", "CPYREF_SCOPE", "CPYREF_NAMESPACE", "CPYREF_PROJECT", "CPYREF_CONFIG", "CPYREF_NO_CONFIG"] {
            cmd.env_remove(key);
        }
        cmd.env("CPYREF_STORE", &self.base).env("CPYREF_NO_CONFIG", "true");
        cmd
    }

    fn run(&self, cwd: &Path, args: &[&str]) -> Output {
        self.command(cwd).args(args).output().unwrap()
    }

    fn copy(&self, cwd: &Path, settings: &[&str]) -> String {
        reference(self.command(cwd).args(settings).args(["copy", "input"]).output().unwrap())
    }

    fn context(&self, cwd: &Path, settings: &[&str]) -> Value {
        let output = self.command(cwd).args(settings).arg("context").output().unwrap();
        serde_json::from_slice(&success(output)).unwrap()
    }
}

#[test]
fn every_reference_operation_is_isolated_between_projects() {
    let f = Fixture::new();
    let id = f.copy(&f.a, &[]);
    for operation in ["paste", "info", "remove"] {
        let error = failure(f.run(&f.b, &[operation, &id]), 6, "REF_NOT_FOUND");
        assert_eq!(error["error"]["details"]["context"]["scope"], "project");
        assert!(!error.to_string().contains("PROJECT_A_SECRET"));
    }
    assert_eq!(success(f.run(&f.a, &["paste", &id])), b"PROJECT_A_SECRET");
}

#[test]
fn same_project_subdirectories_share_default_space() {
    let f = Fixture::new();
    let id = f.copy(&f.a, &[]);
    assert_eq!(success(f.run(&f.a.join("src"), &["paste", &id])), b"PROJECT_A_SECRET");
    assert_eq!(f.context(&f.a, &[])["store_path"], f.context(&f.a.join("src"), &[])["store_path"]);
}

#[test]
fn source_path_does_not_choose_the_clipboard_project() {
    let f = Fixture::new();
    let id = reference(f.command(&f.a).arg("copy").arg(f.b.join("input")).output().unwrap());
    assert_eq!(success(f.run(&f.a, &["paste", &id])), b"PROJECT_B_SECRET");
    failure(f.run(&f.b, &["paste", &id]), 6, "REF_NOT_FOUND");
}

#[test]
fn project_override_does_not_change_input_or_output_paths() {
    let f = Fixture::new();
    let id = reference(f.command(&f.b).arg("--project").arg(&f.a).args(["copy", "input"]).output().unwrap());
    assert_eq!(success(f.run(&f.a, &["paste", &id])), b"PROJECT_B_SECRET");
    success(f.command(&f.b).arg("--project").arg(&f.a).args(["paste", &id, "--output", "result"]).output().unwrap());
    assert_eq!(fs::read(f.b.join("result")).unwrap(), b"PROJECT_B_SECRET");
    assert!(!f.a.join("result").exists());
}

#[test]
fn project_namespaces_isolate_agents_but_can_be_reused_by_collaborators() {
    let f = Fixture::new();
    let id = f.copy(&f.a, &["--namespace", "review"]);
    failure(f.run(&f.a, &["paste", &id]), 6, "REF_NOT_FOUND");
    failure(f.run(&f.a, &["paste", &id, "--namespace", "writer"]), 6, "REF_NOT_FOUND");
    failure(f.run(&f.b, &["paste", &id, "--namespace", "review"]), 6, "REF_NOT_FOUND");
    assert_eq!(success(f.run(&f.a.join("src"), &["paste", &id, "--namespace", "review"])), b"PROJECT_A_SECRET");
}

#[test]
fn explicit_shared_spaces_allow_only_the_selected_team() {
    let f = Fixture::new();
    let id = f.copy(&f.a, &["--scope", "shared", "--namespace", "team"]);
    assert_eq!(success(f.run(&f.b, &["paste", &id, "--scope", "shared", "--namespace", "team"])), b"PROJECT_A_SECRET");
    failure(f.run(&f.b, &["paste", &id, "--namespace", "team"]), 6, "REF_NOT_FOUND");
    failure(f.run(&f.b, &["paste", &id, "--scope", "shared", "--namespace", "other"]), 6, "REF_NOT_FOUND");
}

#[test]
fn legacy_global_snapshots_are_opt_in_and_never_implicitly_migrated() {
    let f = Fixture::new();
    let id = f.copy(&f.a, &["--scope", "global"]);
    assert!(f.base.join(format!("{id}.ref")).is_file());
    for operation in ["paste", "info", "remove"] {
        failure(f.run(&f.a, &[operation, &id]), 6, "REF_NOT_FOUND");
    }
    assert!(!f.base.join("projects").exists());
    assert_eq!(success(f.run(&f.b, &["paste", &id, "--scope", "global"])), b"PROJECT_A_SECRET");
    assert!(f.base.join(format!("{id}.ref")).is_file());
}

#[test]
fn directory_scope_is_distinct_from_project_and_child_directories() {
    let f = Fixture::new();
    let id = f.copy(&f.a, &["--scope", "directory"]);
    failure(f.run(&f.a, &["paste", &id]), 6, "REF_NOT_FOUND");
    failure(f.run(&f.a.join("src"), &["paste", &id, "--scope", "directory"]), 6, "REF_NOT_FOUND");
    assert_eq!(success(f.run(&f.a, &["paste", &id, "--scope", "directory"])), b"PROJECT_A_SECRET");
}

#[test]
fn context_is_metadata_only_and_creates_no_files() {
    let f = Fixture::new();
    let context = f.context(&f.a, &[]);
    assert_eq!(context["scope"], "project");
    assert_eq!(context["namespace"], "default");
    assert!(!context.to_string().contains("PROJECT_A_SECRET"));
    assert!(!f.base.exists());
    assert_eq!(fs::read_dir(&f.a).unwrap().count(), 3);
    failure(f.run(&f.b, &["remove", "c123456789abc"]), 6, "REF_NOT_FOUND");
    assert!(!f.base.exists());
}

#[test]
fn cli_overrides_environment_and_environment_overrides_config() {
    let f = Fixture::new();
    let config = f.dir.path().join("settings.toml");
    fs::write(&config, "scope = 'project'\nnamespace = 'file'\n").unwrap();
    let mut cmd = f.command(&f.a);
    cmd.env_remove("CPYREF_NO_CONFIG").env("CPYREF_CONFIG", &config)
        .env("CPYREF_SCOPE", "shared").env("CPYREF_NAMESPACE", "environment");
    let output = cmd.arg("context").output().unwrap();
    let context: Value = serde_json::from_slice(&success(output)).unwrap();
    assert_eq!(context["scope"], "shared");
    assert_eq!(context["namespace"], "environment");
    let output = cmd.args(["--scope", "project", "--namespace", "flag"]).output().unwrap();
    let context: Value = serde_json::from_slice(&success(output)).unwrap();
    assert_eq!(context["scope"], "project");
    assert_eq!(context["namespace"], "flag");
}

#[test]
fn explicit_config_can_share_and_resolve_store_relative_to_the_file() {
    let f = Fixture::new();
    let config = f.dir.path().join("shared.toml");
    fs::write(&config, "scope = 'shared'\nnamespace = 'team'\nstore = 'configured-store'\n").unwrap();
    let mut a = f.command(&f.a);
    a.env_remove("CPYREF_NO_CONFIG").env_remove("CPYREF_STORE").env("CPYREF_CONFIG", &config);
    let id = reference(a.args(["copy", "input"]).output().unwrap());
    let output = f.command(&f.b).env_remove("CPYREF_NO_CONFIG").env_remove("CPYREF_STORE")
        .env("CPYREF_CONFIG", &config).args(["paste", &id]).output().unwrap();
    assert_eq!(success(output), b"PROJECT_A_SECRET");
    assert!(f.dir.path().join("configured-store").is_dir());
    assert!(!f.base.exists());
}

#[test]
fn malformed_config_prevents_writes_and_does_not_fall_back() {
    let f = Fixture::new();
    let config = f.dir.path().join("bad.toml");
    fs::write(&config, "namespace = [").unwrap();
    let output = f.command(&f.a).env_remove("CPYREF_NO_CONFIG").env("CPYREF_CONFIG", &config)
        .args(["copy", "input"]).output().unwrap();
    failure(output, 4, "CONFIG_ERROR");
    assert!(!f.base.exists());
}

#[test]
fn invalid_scope_namespace_and_conflicting_config_flags_have_no_side_effects() {
    let f = Fixture::new();
    for settings in [vec!["--scope", "wrong"], vec!["--scope", "shared"], vec!["--namespace", "../escape"], vec!["--config", "chosen.toml"]] {
        let output = f.command(&f.a).args(settings).args(["copy", "input"]).output().unwrap();
        failure(output, 4, "INVALID_ARGUMENT");
        assert!(!f.base.exists());
    }
}

#[test]
fn a_storage_base_that_is_a_file_reports_unsafe_store() {
    let f = Fixture::new();
    fs::write(&f.base, b"keep this").unwrap();
    failure(f.run(&f.a, &["copy", "input"]), 5, "UNSAFE_STORE");
    assert_eq!(fs::read(&f.base).unwrap(), b"keep this");
}

#[test]
fn shared_concurrent_agents_publish_distinct_complete_snapshots() {
    let f = Fixture::new();
    let children: Vec<_> = (0..12).map(|i| {
        let cwd = if i % 2 == 0 { &f.a } else { &f.b };
        f.command(cwd).args(["--scope", "shared", "--namespace", "team", "copy", "input"])
            .stdout(Stdio::piped()).stderr(Stdio::piped()).spawn().unwrap()
    }).collect();
    let mut ids = HashSet::new();
    for (i, child) in children.into_iter().enumerate() {
        let id = reference(child.wait_with_output().unwrap());
        assert!(ids.insert(id.clone()));
        let bytes = success(f.run(&f.b, &["--scope", "shared", "--namespace", "team", "paste", &id]));
        assert_eq!(bytes, if i % 2 == 0 { b"PROJECT_A_SECRET" } else { b"PROJECT_B_SECRET" });
    }
}

#[test]
fn portable_namespace_paths_handle_windows_device_names() {
    let f = Fixture::new();
    for name in ["con", "nul", "aux", "com1", "lpt1"] {
        let id = f.copy(&f.a, &["--scope", "shared", "--namespace", name]);
        assert_eq!(success(f.run(&f.b, &["paste", &id, "--scope", "shared", "--namespace", name])), b"PROJECT_A_SECRET");
    }
}

#[cfg(unix)]
#[test]
fn symlink_in_managed_store_path_is_rejected_before_read_or_write() {
    use std::os::unix::fs::symlink;
    let f = Fixture::new();
    let id = f.copy(&f.a, &[]);
    let projects = f.base.join("projects");
    let moved = f.dir.path().join("moved-projects");
    fs::rename(&projects, &moved).unwrap();
    symlink(&moved, &projects).unwrap();
    failure(f.run(&f.a, &["paste", &id]), 5, "UNSAFE_STORE");
    failure(f.run(&f.a, &["copy", "input"]), 5, "UNSAFE_STORE");
    assert!(moved.is_dir());
}

#[cfg(unix)]
#[test]
fn permissive_managed_ancestor_is_rejected() {
    use std::os::unix::fs::PermissionsExt;
    let f = Fixture::new();
    let id = f.copy(&f.a, &[]);
    fs::set_permissions(f.base.join("projects"), fs::Permissions::from_mode(0o755)).unwrap();
    failure(f.run(&f.a, &["paste", &id]), 5, "UNSAFE_STORE");
}

#[test]
fn inventory_stats_and_cleanup_never_cross_project_or_namespace() {
    let f = Fixture::new();
    let id = f.copy(&f.a, &[]);
    let other = f.copy(&f.a, &["--namespace", "other"]);
    assert_eq!(serde_json::from_slice::<Value>(&success(f.run(&f.b, &["list", "--json"]))).unwrap()["total"], 0);
    assert_eq!(serde_json::from_slice::<Value>(&success(f.run(&f.b, &["stats"]))).unwrap()["count"], 0);
    let preview: Value = serde_json::from_slice(&success(f.run(&f.b, &["clean", "--all"]))).unwrap();
    assert_eq!(preview["count"], 0);
    success(f.run(&f.b, &["clean", "--all", "--apply", "--confirm", preview["confirm"].as_str().unwrap()]));
    assert_eq!(success(f.run(&f.a, &["paste", &id])), b"PROJECT_A_SECRET");
    let preview: Value = serde_json::from_slice(&success(f.run(&f.a, &["clean", "--all"]))).unwrap();
    assert_eq!(preview["count"], 1);
    success(f.run(&f.a, &["clean", "--all", "--apply", "--confirm", preview["confirm"].as_str().unwrap()]));
    assert_eq!(success(f.run(&f.a, &["--namespace", "other", "paste", &other])), b"PROJECT_A_SECRET");
}
