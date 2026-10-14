//! Resolve one clipboard space before any snapshot is read or written.
use crate::{
    error::{Error, Result},
    store::Store,
};
use clap::{Args, ValueEnum};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::{
    fs,
    io::Read,
    path::{Path, PathBuf},
};

const PROJECT_CONFIG: &str = ".cpyref.toml";
const MAX_CONFIG_BYTES: u64 = 64 * 1024;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, ValueEnum)]
#[serde(rename_all = "lowercase")]
pub enum Scope {
    Project,
    Directory,
    Shared,
    Global,
}

#[derive(Debug, Default, Args)]
pub struct Options {
    /// Maximum references per selected space; 0 means unlimited.
    #[arg(long, global = true, env = "CPYREF_MAX_ENTRIES")]
    pub max_entries: Option<usize>,
    /// Storage base. Project isolation still applies; global scope uses this path directly.
    #[arg(long, global = true, env = "CPYREF_STORE")]
    pub store: Option<PathBuf>,
    /// Clipboard scope. Default: project. Shared/global access must be explicitly configured.
    #[arg(long, global = true, value_enum, env = "CPYREF_SCOPE")]
    pub scope: Option<Scope>,
    /// Space within the scope, e.g. default, agent-a, or review-team. Required for shared scope.
    #[arg(long, global = true, env = "CPYREF_NAMESPACE")]
    pub namespace: Option<String>,
    /// Use this existing directory as the project root; does not change input/output paths.
    #[arg(long, global = true, env = "CPYREF_PROJECT")]
    pub project: Option<PathBuf>,
    /// Explicit trusted TOML config; replaces automatic user and project config files.
    #[arg(
        long,
        global = true,
        env = "CPYREF_CONFIG",
        conflicts_with = "no_config"
    )]
    pub config: Option<PathBuf>,
    /// Ignore config file settings. Project marker discovery still applies.
    #[arg(long, global = true, env = "CPYREF_NO_CONFIG")]
    pub no_config: bool,
}

#[derive(Debug, Default, Deserialize)]
#[serde(deny_unknown_fields)]
struct Config {
    #[serde(default)]
    cleanup: CleanupConfig,
    #[serde(default)]
    run: crate::runner::RunConfig,
    max_entries: Option<usize>,
    scope: Option<Scope>,
    namespace: Option<String>,
    store: Option<PathBuf>,
}

#[derive(Debug, Default, Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct CleanupConfig {
    pub older_than_days: Option<u32>,
    pub keep_latest: Option<usize>,
}

#[derive(Debug)]
pub struct Context {
    pub run: crate::runner::RunConfig,
    pub cleanup: CleanupConfig,
    pub max_entries: usize,
    pub scope: Scope,
    pub namespace: String,
    pub cwd: PathBuf,
    pub project_root: Option<PathBuf>,
    pub project_source: Option<&'static str>,
    pub store_base: PathBuf,
    pub store_path: PathBuf,
    pub config_files: Vec<PathBuf>,
}

fn config_error(path: &Path, message: impl Into<String>) -> Error {
    Error::new("CONFIG_ERROR", 4, message).details(json!({"path": path.to_string_lossy()}))
}

fn valid_namespace(name: &str) -> bool {
    let bytes = name.as_bytes();
    !bytes.is_empty()
        && bytes.len() <= 64
        && (bytes[0].is_ascii_lowercase() || bytes[0].is_ascii_digit())
        && bytes
            .iter()
            .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || *b == b'-' || *b == b'_')
}

fn read_config(path: &Path, required: bool) -> Result<Option<Config>> {
    let metadata = match fs::symlink_metadata(path) {
        Ok(metadata) => metadata,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound && !required => return Ok(None),
        Err(e) => return Err(config_error(path, format!("Cannot read config file: {e}"))),
    };
    if !metadata.is_file() || metadata.file_type().is_symlink() {
        return Err(config_error(
            path,
            "Config must be a regular file, not a symbolic link.",
        ));
    }
    let mut bytes = Vec::new();
    fs::File::open(path)
        .and_then(|file| file.take(MAX_CONFIG_BYTES + 1).read_to_end(&mut bytes))
        .map_err(|e| config_error(path, format!("Cannot read config file: {e}")))?;
    if bytes.len() as u64 > MAX_CONFIG_BYTES {
        return Err(config_error(path, "Config exceeds the 64 KiB limit."));
    }
    let text = std::str::from_utf8(&bytes)
        .map_err(|_| config_error(path, "Config must contain UTF-8 text."))?;
    let mut config: Config = toml::from_str(text).map_err(|_| {
        // Do not echo configuration values, which could contain private paths or secrets.
        config_error(path, "Invalid TOML or unknown fields. Supported fields: scope, namespace, store, max_entries, cleanup (older_than_days, keep_latest), run (max_log_bytes, timeout_seconds, min_free_bytes).")
    })?;
    config.run.clone().resolve().map_err(|_| config_error(path, "Invalid run limits: max_log_bytes must be 1..1073741824; timeout_seconds must be 1..86400."))?;
    if config
        .namespace
        .as_deref()
        .is_some_and(|name| !valid_namespace(name))
    {
        return Err(config_error(path, "Namespace must be 1-64 lowercase ASCII letters, digits, - or _, starting with a letter or digit."));
    }
    if let Some(store) = &mut config.store {
        if store.as_os_str().is_empty() {
            return Err(config_error(path, "Store path must not be empty."));
        }
        // Relative file settings are anchored to the file, never the caller's subdirectory.
        *store = path
            .parent()
            .unwrap_or_else(|| Path::new("."))
            .join(&*store);
    }
    Ok(Some(config))
}

fn directory(path: &Path) -> Result<PathBuf> {
    if path.as_os_str().is_empty() {
        return Err(Error::new(
            "PROJECT_ERROR",
            4,
            "Project directory must not be empty.",
        ));
    }
    let canonical = fs::canonicalize(path).map_err(|e| {
        Error::new(
            "PROJECT_ERROR",
            4,
            format!("Cannot resolve project directory: {e}"),
        )
        .details(json!({"path": path.to_string_lossy()}))
    })?;
    if !canonical.is_dir() {
        return Err(Error::new(
            "PROJECT_ERROR",
            4,
            "Project root must be an existing directory.",
        ));
    }
    Ok(canonical)
}

fn discover(cwd: &Path) -> Result<(PathBuf, &'static str)> {
    // A .git file marks a worktree/submodule just as a .git directory marks a checkout.
    // Never execute Git or derive identity from remotes, branch names, or GIT_DIR.
    for parent in cwd.ancestors() {
        for (name, source) in [(PROJECT_CONFIG, "project-config"), (".git", "git")] {
            let marker = parent.join(name);
            match fs::symlink_metadata(&marker) {
                Ok(meta) => {
                    let valid_type = meta.is_file() || (name == ".git" && meta.is_dir());
                    if !valid_type || meta.file_type().is_symlink() {
                        return Err(Error::new(
                            "PROJECT_ERROR",
                            4,
                            "Project marker must be a real file or .git directory.",
                        )
                        .details(json!({"path": marker.to_string_lossy()})));
                    }
                    return Ok((parent.to_path_buf(), source));
                }
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
                Err(e) => return Err(Error::io("Inspect project marker", e)),
            }
        }
    }
    Ok((cwd.to_path_buf(), "cwd"))
}

fn identity(scope: Scope, path: &Path) -> String {
    let mut hash = Sha256::new();
    hash.update(match scope {
        Scope::Project => b"cpyref-project-v1\0".as_slice(),
        _ => b"cpyref-directory-v1\0".as_slice(),
    });
    // Lossy display strings could merge different non-UTF-8 directory names.
    hash.update(path.as_os_str().as_encoded_bytes());
    format!("{:x}", hash.finalize())
}

impl Context {
    pub fn resolve(options: &Options) -> Result<Self> {
        let cwd = std::env::current_dir().map_err(|e| Error::io("Read current directory", e))?;
        let user_config = dirs::config_dir().map(|p| p.join("cpyref").join("config.toml"));
        Self::resolve_at(options, &cwd, user_config.as_deref())
    }

    /// Explicit paths make resolution testable without changing process-wide environment or cwd.
    pub fn resolve_at(options: &Options, cwd: &Path, user_config: Option<&Path>) -> Result<Self> {
        if options.no_config && options.config.is_some() {
            return Err(Error::invalid(
                "--config and --no-config cannot be combined.",
            ));
        }
        let cwd = directory(cwd)?;
        let (project, project_source) = match &options.project {
            Some(path) if path.as_os_str().is_empty() => {
                return Err(Error::new(
                    "PROJECT_ERROR",
                    4,
                    "Project directory must not be empty.",
                ));
            }
            Some(path) => (directory(&cwd.join(path))?, "explicit"),
            None => discover(&cwd)?,
        };
        let mut config_files = Vec::new();
        let mut trusted = Config::default();
        let mut local = Config::default();
        if !options.no_config {
            let explicit = options.config.as_ref().map(|p| cwd.join(p));
            let path = explicit.as_deref().or(user_config);
            if let Some(path) = path {
                let path = cwd.join(path);
                if let Some(config) = read_config(&path, explicit.is_some())? {
                    trusted = config;
                    config_files.push(path);
                }
            }
            if explicit.is_none() {
                let path = project.join(PROJECT_CONFIG);
                if let Some(config) = read_config(&path, false)? {
                    // Repository-controlled files must not silently redirect data or enable sharing.
                    if config.store.is_some()
                        || matches!(config.scope, Some(Scope::Shared | Scope::Global))
                    {
                        return Err(config_error(&path, "Automatic project config may set only project/directory scope, namespace, max_entries, and cleanup defaults. For sharing or a custom store, explicitly select a trusted --config file or use CLI/environment settings."));
                    }
                    local = config;
                    config_files.push(path);
                }
            }
        }
        let scope = options
            .scope
            .or(local.scope)
            .or(trusted.scope)
            .unwrap_or(Scope::Project);
        if options.project.is_some() && scope != Scope::Project {
            return Err(Error::invalid("--project/CPYREF_PROJECT requires project scope; it is not used by directory, shared, or global scope."));
        }
        // Repository settings are never used to select a cross-project space.
        let local_namespace = if matches!(scope, Scope::Project | Scope::Directory) {
            local.namespace
        } else {
            None
        };
        let namespace = options
            .namespace
            .clone()
            .or(local_namespace)
            .or(trusted.namespace);
        if scope == Scope::Shared && namespace.is_none() {
            return Err(Error::invalid("Shared scope requires an explicit --namespace/CPYREF_NAMESPACE or a namespace in the trusted config."));
        }
        let namespace = namespace.unwrap_or_else(|| "default".to_owned());
        if !valid_namespace(&namespace) {
            return Err(Error::invalid("Namespace must be 1-64 lowercase ASCII letters, digits, - or _, starting with a letter or digit."));
        }
        if scope == Scope::Global && namespace != "default" {
            return Err(Error::invalid("Global scope is the legacy flat store and only supports namespace default. Use shared scope for named cross-project spaces."));
        }
        let base = options.store.clone().or(trusted.store);
        let base = match base {
            Some(path) if path.as_os_str().is_empty() => {
                return Err(Error::invalid("Store path must not be empty."))
            }
            Some(path) => cwd.join(path),
            None => cwd.join(Store::default_root()?),
        };
        // Prefix names so valid namespaces such as con/nul/com1 are safe on Windows too.
        let directory_name = format!("ns-{namespace}");
        let store_path = match scope {
            Scope::Project => base
                .join("projects")
                .join(identity(scope, &project))
                .join(&directory_name),
            Scope::Directory => base
                .join("directories")
                .join(identity(scope, &cwd))
                .join(&directory_name),
            Scope::Shared => base.join("shared").join(&directory_name),
            Scope::Global => base.clone(),
        };
        let local_limit = if matches!(scope, Scope::Project | Scope::Directory) {
            local.max_entries
        } else {
            None
        };
        let cleanup = if matches!(scope, Scope::Project | Scope::Directory) {
            CleanupConfig {
                older_than_days: local
                    .cleanup
                    .older_than_days
                    .or(trusted.cleanup.older_than_days),
                keep_latest: local.cleanup.keep_latest.or(trusted.cleanup.keep_latest),
            }
        } else {
            trusted.cleanup
        };
        let run = if matches!(scope, Scope::Project | Scope::Directory) {
            local.run.merge(trusted.run)
        } else {
            trusted.run
        };
        Ok(Self {
            run,
            cleanup,
            max_entries: options
                .max_entries
                .or(local_limit)
                .or(trusted.max_entries)
                .unwrap_or(0),
            scope,
            namespace,
            cwd,
            project_root: (scope == Scope::Project).then_some(project),
            project_source: (scope == Scope::Project).then_some(project_source),
            store_base: base,
            store_path,
            config_files,
        })
    }

    pub fn store(&self) -> Result<Store> {
        Store::scoped(self.store_base.clone(), self.store_path.clone())
    }

    /// Diagnostics contain locations and settings, never a snapshot body. This does not create files.
    pub fn report(&self) -> Value {
        json!({
            "run": self.run.clone().resolve().ok(),
            "cleanup": self.cleanup,
            "max_entries": self.max_entries,
            "scope": self.scope,
            "namespace": self.namespace,
            "cwd": self.cwd.to_string_lossy(),
            "project_root": self.project_root.as_ref().map(|p| p.to_string_lossy()),
            "project_source": self.project_source,
            "store_base": self.store_base.to_string_lossy(),
            "store_path": self.store_path.to_string_lossy(),
            "config_files": self.config_files.iter().map(|p| p.to_string_lossy()).collect::<Vec<_>>()
        })
    }

    pub fn annotate(&self, error: Error) -> Error {
        if error.code == "REF_NOT_FOUND" {
            error.details(json!({
                "context": self.report(),
                "hint": "No reference in the selected space. Use cpyref context and confirm the original project/scope/namespace. Other spaces and the legacy global store are never searched automatically."
            }))
        } else {
            error
        }
    }
}

#[cfg(all(test, unix))]
mod tests {
    use super::{identity, Scope};
    use std::{ffi::OsStr, os::unix::ffi::OsStrExt, path::Path};

    #[test]
    fn path_identity_preserves_non_utf8_bytes_without_filesystem_access() {
        // macOS filesystems may reject these names. Hashing must still use native bytes,
        // not a lossy display string, on every Unix platform.
        let a = Path::new(OsStr::from_bytes(b"/project-\xff"));
        let b = Path::new(OsStr::from_bytes(b"/project-\xfe"));
        assert_eq!(a.to_string_lossy(), b.to_string_lossy());
        for scope in [Scope::Project, Scope::Directory] {
            assert_ne!(identity(scope, a), identity(scope, b));
            assert_eq!(identity(scope, a).len(), 64);
        }
        assert_ne!(identity(Scope::Project, a), identity(Scope::Directory, a));
    }
}
