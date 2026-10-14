use cpyref::context::{Context, Options, Scope};
use std::{
    fs,
    path::{Path, PathBuf},
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

    fn folder(&self, path: &str) -> PathBuf {
        let path = self.dir.path().join(path);
        fs::create_dir_all(&path).unwrap();
        path
    }

    fn repo(&self, path: &str) -> PathBuf {
        let path = self.folder(path);
        fs::create_dir(path.join(".git")).unwrap();
        path
    }

    fn options(&self) -> Options {
        Options {
            store: Some(self.dir.path().join("store")),
            no_config: true,
            ..Options::default()
        }
    }

    fn context(&self, cwd: &Path) -> Context {
        Context::resolve_at(&self.options(), cwd, None).unwrap()
    }
}

#[test]
fn default_project_unifies_subdirectories_and_canonical_paths() {
    let f = Fixture::new();
    let root = f.repo("project");
    let child = f.folder("project/src/nested");
    let a = f.context(&root);
    let b = f.context(&child.join(".."));
    assert_eq!(a.scope, Scope::Project);
    assert_eq!(a.namespace, "default");
    assert_eq!(a.project_root, Some(fs::canonicalize(&root).unwrap()));
    assert_eq!(a.store_path, b.store_path);
    assert_eq!(a.project_source, Some("git"));
    assert!(!a.store_base.exists());
}

#[test]
fn same_basename_does_not_merge_distinct_projects() {
    let f = Fixture::new();
    let a = f.repo("a/app");
    let b = f.repo("b/app");
    assert_ne!(f.context(&a).store_path, f.context(&b).store_path);
}

#[test]
fn unmarked_folders_fall_back_to_exact_cwd() {
    let f = Fixture::new();
    let root = f.folder("plain");
    let child = f.folder("plain/sub");
    assert_eq!(f.context(&root).project_source, Some("cwd"));
    assert_ne!(f.context(&root).store_path, f.context(&child).store_path);
}

#[test]
fn empty_config_marks_a_non_git_project_even_with_no_config() {
    let f = Fixture::new();
    let root = f.folder("plain");
    let child = f.folder("plain/sub");
    fs::write(root.join(".cpyref.toml"), "").unwrap();
    assert_eq!(f.context(&root).store_path, f.context(&child).store_path);
    assert_eq!(f.context(&child).project_source, Some("project-config"));
}

#[test]
fn nested_git_and_config_markers_are_project_boundaries() {
    let f = Fixture::new();
    let root = f.repo("outer");
    let git = f.repo("outer/inner");
    let marked = f.folder("outer/package");
    fs::write(marked.join(".cpyref.toml"), "").unwrap();
    assert_ne!(f.context(&root).store_path, f.context(&git).store_path);
    assert_ne!(f.context(&root).store_path, f.context(&marked).store_path);
}

#[test]
fn git_files_keep_worktrees_separate_by_default() {
    let f = Fixture::new();
    let a = f.folder("worktree-a");
    let b = f.folder("worktree-b");
    fs::write(a.join(".git"), "gitdir: /unused/common/worktrees/a\n").unwrap();
    fs::write(b.join(".git"), "gitdir: /unused/common/worktrees/b\n").unwrap();
    assert_eq!(f.context(&a).project_source, Some("git"));
    assert_ne!(f.context(&a).store_path, f.context(&b).store_path);
}

#[test]
fn directory_scope_splits_subdirectories() {
    let f = Fixture::new();
    let root = f.repo("project");
    let child = f.folder("project/sub");
    let options = Options {
        scope: Some(Scope::Directory),
        ..f.options()
    };
    let a = Context::resolve_at(&options, &root, None).unwrap();
    let b = Context::resolve_at(&options, &child, None).unwrap();
    assert_ne!(a.store_path, b.store_path);
    assert_ne!(a.store_path, f.context(&root).store_path);
    assert!(a.project_root.is_none());
}

#[test]
fn explicit_project_pins_context_without_changing_cwd() {
    let f = Fixture::new();
    let root = f.repo("project");
    let outside = f.folder("outside");
    let options = Options {
        project: Some(root.clone()),
        ..f.options()
    };
    let context = Context::resolve_at(&options, &outside, None).unwrap();
    assert_eq!(context.store_path, f.context(&root).store_path);
    assert_eq!(context.cwd, fs::canonicalize(outside).unwrap());
    assert_eq!(context.project_source, Some("explicit"));
}

#[test]
fn invalid_project_never_falls_back() {
    let f = Fixture::new();
    let cwd = f.repo("project");
    let file = cwd.join("file");
    fs::write(&file, "data").unwrap();
    for path in [cwd.join("missing"), file, PathBuf::new()] {
        let options = Options {
            project: Some(path),
            ..f.options()
        };
        assert_eq!(
            Context::resolve_at(&options, &cwd, None).unwrap_err().code,
            "PROJECT_ERROR"
        );
    }
    let options = Options {
        scope: Some(Scope::Shared),
        project: Some(cwd.clone()),
        namespace: Some("team".into()),
        ..f.options()
    };
    assert_eq!(
        Context::resolve_at(&options, &cwd, None).unwrap_err().code,
        "INVALID_ARGUMENT"
    );
}

#[test]
fn namespaces_are_project_local_unless_shared_is_selected() {
    let f = Fixture::new();
    let a = f.repo("a");
    let b = f.repo("b");
    let options = Options {
        namespace: Some("team".into()),
        ..f.options()
    };
    let local = Context::resolve_at(&options, &a, None).unwrap();
    assert_ne!(
        local.store_path,
        Context::resolve_at(&options, &b, None).unwrap().store_path
    );
    let options = Options {
        scope: Some(Scope::Shared),
        ..options
    };
    let shared = Context::resolve_at(&options, &a, None).unwrap();
    assert_eq!(
        shared.store_path,
        Context::resolve_at(&options, &b, None).unwrap().store_path
    );
    assert_ne!(shared.store_path, local.store_path);
}

#[test]
fn invalid_names_and_unnamed_sharing_are_rejected() {
    let f = Fixture::new();
    let cwd = f.repo("project");
    for name in [
        "",
        "../escape",
        "a/b",
        "a\\b",
        "UPPER",
        ".",
        "two words",
        "中文",
        &"x".repeat(65),
    ] {
        let options = Options {
            namespace: Some(name.into()),
            ..f.options()
        };
        assert_eq!(
            Context::resolve_at(&options, &cwd, None).unwrap_err().code,
            "INVALID_ARGUMENT"
        );
    }
    let options = Options {
        scope: Some(Scope::Shared),
        ..f.options()
    };
    assert_eq!(
        Context::resolve_at(&options, &cwd, None).unwrap_err().code,
        "INVALID_ARGUMENT"
    );
}

#[test]
fn global_scope_is_exact_legacy_base_and_rejects_named_spaces() {
    let f = Fixture::new();
    let cwd = f.repo("project");
    let mut options = Options {
        scope: Some(Scope::Global),
        ..f.options()
    };
    let context = Context::resolve_at(&options, &cwd, None).unwrap();
    assert_eq!(context.store_path, context.store_base);
    options.namespace = Some("team".into());
    assert_eq!(
        Context::resolve_at(&options, &cwd, None).unwrap_err().code,
        "INVALID_ARGUMENT"
    );
}

#[test]
fn project_config_overrides_user_settings_and_options_override_both() {
    let f = Fixture::new();
    let root = f.repo("project");
    let sub = f.folder("project/sub");
    let user = f.dir.path().join("user.toml");
    fs::write(&user, "namespace = 'user'\nscope = 'directory'\n").unwrap();
    fs::write(
        root.join(".cpyref.toml"),
        "namespace = 'project-team'\nscope = 'project'\n",
    )
    .unwrap();
    let mut options = Options {
        no_config: false,
        ..f.options()
    };
    let context = Context::resolve_at(&options, &sub, Some(&user)).unwrap();
    assert_eq!(context.scope, Scope::Project);
    assert_eq!(context.namespace, "project-team");
    assert_eq!(context.config_files.len(), 2);
    options.namespace = Some("flag".into());
    options.scope = Some(Scope::Directory);
    let context = Context::resolve_at(&options, &sub, Some(&user)).unwrap();
    assert_eq!(context.scope, Scope::Directory);
    assert_eq!(context.namespace, "flag");
}

#[test]
fn explicit_config_replaces_automatic_files_and_anchors_relative_store() {
    let f = Fixture::new();
    let root = f.repo("project");
    let sub = f.folder("project/sub");
    fs::write(root.join(".cpyref.toml"), "invalid = [").unwrap();
    let user = f.dir.path().join("user.toml");
    fs::write(&user, "invalid = [").unwrap();
    let settings = f.folder("settings");
    let chosen = settings.join("team.toml");
    fs::write(
        &chosen,
        "scope = 'shared'\nnamespace = 'build'\nstore = 'data'\n",
    )
    .unwrap();
    let options = Options {
        config: Some(chosen),
        ..Options::default()
    };
    let a = Context::resolve_at(&options, &root, Some(&user)).unwrap();
    let b = Context::resolve_at(&options, &sub, Some(&user)).unwrap();
    assert_eq!(a.scope, Scope::Shared);
    assert_eq!(a.namespace, "build");
    assert_eq!(a.store_base, settings.join("data"));
    assert_eq!(a.store_path, b.store_path);
    assert_eq!(a.config_files.len(), 1);
    assert!(!a.store_base.exists());
}

#[test]
fn automatic_project_config_cannot_redirect_or_enable_cross_project_sharing() {
    let f = Fixture::new();
    let root = f.repo("project");
    let options = Options {
        no_config: false,
        ..f.options()
    };
    for text in [
        "scope = 'shared'\nnamespace = 'other'",
        "scope = 'global'",
        "store = '../other'",
    ] {
        fs::write(root.join(".cpyref.toml"), text).unwrap();
        assert_eq!(
            Context::resolve_at(&options, &root, None).unwrap_err().code,
            "CONFIG_ERROR"
        );
    }
    assert!(!f.dir.path().join("store").exists());
}

#[test]
fn local_namespace_cannot_redirect_an_explicit_shared_space() {
    let f = Fixture::new();
    let root = f.repo("project");
    fs::write(root.join(".cpyref.toml"), "namespace = 'repository-choice'").unwrap();
    let user = f.dir.path().join("user.toml");
    fs::write(&user, "namespace = 'trusted-team'").unwrap();
    let options = Options {
        no_config: false,
        scope: Some(Scope::Shared),
        ..f.options()
    };
    let context = Context::resolve_at(&options, &root, Some(&user)).unwrap();
    assert_eq!(context.namespace, "trusted-team");
}

#[test]
fn no_config_ignores_settings_but_not_project_markers() {
    let f = Fixture::new();
    let root = f.folder("plain");
    let sub = f.folder("plain/sub");
    fs::write(root.join(".cpyref.toml"), "bad syntax [").unwrap();
    let context = f.context(&sub);
    assert_eq!(context.namespace, "default");
    assert_eq!(context.project_root, Some(fs::canonicalize(root).unwrap()));
    assert!(context.config_files.is_empty());
}

#[test]
fn configuration_search_stops_at_the_nearest_git_boundary() {
    let f = Fixture::new();
    let outer = f.repo("outer");
    let inner = f.repo("outer/inner");
    fs::write(outer.join(".cpyref.toml"), "invalid = [").unwrap();
    let options = Options {
        no_config: false,
        ..f.options()
    };
    let context = Context::resolve_at(&options, &inner, None).unwrap();
    assert_eq!(context.project_root, Some(fs::canonicalize(inner).unwrap()));
    assert!(context.config_files.is_empty());
}

#[test]
fn malformed_oversized_and_missing_explicit_configs_fail_closed() {
    let f = Fixture::new();
    let cwd = f.repo("project");
    let config = f.dir.path().join("config.toml");
    let options = Options {
        config: Some(config.clone()),
        no_config: false,
        ..f.options()
    };
    assert_eq!(
        Context::resolve_at(&options, &cwd, None).unwrap_err().code,
        "CONFIG_ERROR"
    );
    for content in [
        b"scope = 'unknown'".to_vec(),
        b"namespcae = 'typo'".to_vec(),
        b"namespace = ''".to_vec(),
        b"store = ''".to_vec(),
        vec![0xff],
        vec![b' '; 65537],
    ] {
        fs::write(&config, content).unwrap();
        assert_eq!(
            Context::resolve_at(&options, &cwd, None).unwrap_err().code,
            "CONFIG_ERROR"
        );
    }
    fs::remove_file(&config).unwrap();
    fs::create_dir(&config).unwrap();
    assert_eq!(
        Context::resolve_at(&options, &cwd, None).unwrap_err().code,
        "CONFIG_ERROR"
    );
}

#[test]
fn explicit_config_and_no_config_are_incompatible_in_library_api() {
    let f = Fixture::new();
    let cwd = f.repo("project");
    let options = Options {
        config: Some(cwd.join("config.toml")),
        ..f.options()
    };
    assert_eq!(
        Context::resolve_at(&options, &cwd, None).unwrap_err().code,
        "INVALID_ARGUMENT"
    );
}

#[cfg(unix)]
#[test]
fn symlink_alias_resolves_to_the_same_project() {
    let f = Fixture::new();
    let root = f.repo("project");
    let alias = f.dir.path().join("alias");
    std::os::unix::fs::symlink(&root, &alias).unwrap();
    assert_eq!(f.context(&root).store_path, f.context(&alias).store_path);
}

// Linux CI can create these names. macOS CI rejects them with EILSEQ.
// A filesystem-independent unit test in context.rs covers native-byte hashing on all Unix.
#[cfg(target_os = "linux")]
#[test]
fn non_utf8_paths_do_not_collapse_into_one_space() {
    use std::{ffi::OsStr, os::unix::ffi::OsStrExt};
    let f = Fixture::new();
    let a = f.dir.path().join(OsStr::from_bytes(b"project-\xff"));
    let b = f.dir.path().join(OsStr::from_bytes(b"project-\xfe"));
    fs::create_dir(&a).unwrap();
    fs::create_dir(&b).unwrap();
    assert_eq!(a.to_string_lossy(), b.to_string_lossy());
    assert_ne!(f.context(&a).store_path, f.context(&b).store_path);
    assert_eq!(f.context(&a).report()["scope"], "project");
}

#[cfg(unix)]
#[test]
fn symlink_config_files_are_rejected() {
    let f = Fixture::new();
    let root = f.repo("project");
    let target = f.dir.path().join("real.toml");
    let alias = f.dir.path().join("alias.toml");
    fs::write(&target, "namespace = 'team'").unwrap();
    std::os::unix::fs::symlink(&target, &alias).unwrap();
    let options = Options {
        config: Some(alias),
        no_config: false,
        ..f.options()
    };
    assert_eq!(
        Context::resolve_at(&options, &root, None).unwrap_err().code,
        "CONFIG_ERROR"
    );
}

#[test]
fn max_entries_uses_config_precedence_and_ignores_local_limit_for_shared_scope() {
    let f = Fixture::new();
    let root = f.repo("project");
    let user = f.dir.path().join("user.toml");
    fs::write(&user, "max_entries = 100\nnamespace = 'trusted'\n").unwrap();
    fs::write(root.join(".cpyref.toml"), "max_entries = 20\n").unwrap();
    let mut options = Options {
        store: Some(f.dir.path().join("store")),
        ..Options::default()
    };
    assert_eq!(
        Context::resolve_at(&options, &root, Some(&user))
            .unwrap()
            .max_entries,
        20
    );
    options.max_entries = Some(0);
    assert_eq!(
        Context::resolve_at(&options, &root, Some(&user))
            .unwrap()
            .max_entries,
        0
    );
    options.max_entries = None;
    options.scope = Some(Scope::Shared);
    assert_eq!(
        Context::resolve_at(&options, &root, Some(&user))
            .unwrap()
            .max_entries,
        100
    );
    options.no_config = true;
    options.scope = None;
    assert_eq!(
        Context::resolve_at(&options, &root, Some(&user))
            .unwrap()
            .max_entries,
        0
    );
}

#[test]
fn invalid_entry_limits_fail_before_writing() {
    let f = Fixture::new();
    let root = f.repo("project");
    let config = root.join(".cpyref.toml");
    for text in [
        "max_entries = -1",
        "max_entries = 1.5",
        "max_entries = '10'",
        "max_entries = true",
    ] {
        fs::write(&config, text).unwrap();
        let options = Options {
            no_config: false,
            ..f.options()
        };
        assert_eq!(
            Context::resolve_at(&options, &root, None).unwrap_err().code,
            "CONFIG_ERROR"
        );
    }
    assert!(!f.dir.path().join("store").exists());
}

#[test]
fn cleanup_defaults_merge_by_field_and_do_not_leak_from_project_to_shared() {
    let f = Fixture::new();
    let root = f.repo("project");
    let user = f.dir.path().join("user.toml");
    fs::write(
        &user,
        "namespace = 'team'\n[cleanup]\nolder_than_days = 30\nkeep_latest = 20\n",
    )
    .unwrap();
    fs::write(root.join(".cpyref.toml"), "[cleanup]\nkeep_latest = 5\n").unwrap();
    let mut options = Options {
        no_config: false,
        ..f.options()
    };
    let c = Context::resolve_at(&options, &root, Some(&user)).unwrap();
    assert_eq!(c.cleanup.older_than_days, Some(30));
    assert_eq!(c.cleanup.keep_latest, Some(5));
    options.scope = Some(Scope::Shared);
    assert_eq!(
        Context::resolve_at(&options, &root, Some(&user))
            .unwrap()
            .cleanup
            .keep_latest,
        Some(20)
    );
    options.no_config = true;
    options.scope = None;
    assert!(Context::resolve_at(&options, &root, Some(&user))
        .unwrap()
        .cleanup
        .keep_latest
        .is_none());
}
