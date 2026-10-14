//! Explicit source manifests; hashes describe observed files, not compiler dependency tracing.
use crate::{
    context::Context,
    error::{Error, Result},
    store::{hash_reader, Metadata, RecordKind, Store},
};
use serde::{Deserialize, Serialize};
use std::{
    collections::BTreeMap,
    fs::{self, File},
    io::Write,
    path::{Component, Path, PathBuf},
};

const MAX_FILES: usize = 10_000;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct Entry {
    pub bytes: u64,
    pub sha256: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Manifest {
    pub schema: u8,
    pub root: PathBuf,
    pub selections: Vec<String>,
    pub excluded: Vec<String>,
    pub files: BTreeMap<String, Entry>,
}

#[derive(Debug, Clone, Serialize)]
pub struct Change {
    pub path: String,
    pub status: &'static str,
}

pub fn native_text(path: &Path) -> Result<String> {
    path.to_str().map(str::to_owned).ok_or_else(|| {
        Error::new(
            "PATH_ENCODING",
            4,
            "This operation requires UTF-8 paths; byte copy still supports native paths.",
        )
    })
}

fn normalize(path: &Path) -> PathBuf {
    let mut out = PathBuf::new();
    for component in path.components() {
        match component {
            Component::ParentDir => {
                out.pop();
            }
            Component::CurDir => {}
            _ => out.push(component),
        }
    }
    out
}

/// Do not follow symlinks in any selected path component, including during later checks.
fn inspect(root: &Path, relative: &str) -> Result<Option<fs::Metadata>> {
    let relative = Path::new(relative);
    if relative.is_absolute()
        || relative
            .components()
            .any(|c| !matches!(c, Component::Normal(_) | Component::CurDir))
    {
        return Err(Error::invalid(
            "Manifest paths must remain within their source root.",
        ));
    }
    let canonical = fs::canonicalize(root).map_err(|e| Error::io("Resolve source root", e))?;
    if canonical != normalize(root) {
        return Err(Error::new(
            "UNSAFE_SOURCE",
            4,
            "Source root contains a redirected path component.",
        ));
    }
    let root_meta = fs::symlink_metadata(root).map_err(|e| Error::io("Inspect source root", e))?;
    if !root_meta.is_dir() || root_meta.file_type().is_symlink() {
        return Err(Error::new(
            "UNSAFE_SOURCE",
            4,
            "Source root is not a real directory.",
        ));
    }
    let mut path = root.to_path_buf();
    let mut last = root_meta;
    for component in relative.components() {
        if component == Component::CurDir {
            continue;
        }
        path.push(component);
        last = match fs::symlink_metadata(&path) {
            Ok(m) => m,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(None),
            Err(e) => return Err(Error::io("Inspect source", e)),
        };
        if last.file_type().is_symlink() {
            return Err(Error::new(
                "UNSAFE_SOURCE",
                4,
                "Source manifests do not follow symbolic links.",
            )
            .details(serde_json::json!({"path": path})));
        }
        #[cfg(windows)]
        {
            use std::os::windows::fs::MetadataExt;
            if last.file_attributes() & 0x400 != 0 {
                return Err(Error::new(
                    "UNSAFE_SOURCE",
                    4,
                    "Source manifests do not follow reparse points.",
                ));
            }
        }
    }
    Ok(Some(last))
}

pub fn hash_file(root: &Path, relative: &str) -> Result<Option<Entry>> {
    let Some(before) = inspect(root, relative)? else {
        return Ok(None);
    };
    if !before.is_file() {
        return Err(Error::new(
            "UNSAFE_SOURCE",
            4,
            "Expected a regular source file.",
        ));
    }
    let mut file = File::open(root.join(relative)).map_err(|e| Error::io("Open source", e))?;
    let sha256 = hash_reader(&mut file)?;
    let after = file
        .metadata()
        .map_err(|e| Error::io("Recheck source", e))?;
    if before.len() != after.len() || before.modified().ok() != after.modified().ok() {
        return Err(Error::new(
            "SOURCE_CHANGED",
            4,
            "A source file changed while it was hashed.",
        ));
    }
    Ok(Some(Entry {
        bytes: after.len(),
        sha256,
    }))
}

fn scan(
    manifest: &Manifest,
    selection: &str,
    files: &mut BTreeMap<String, Entry>,
    allow_missing: bool,
) -> Result<()> {
    let path = Path::new(selection);
    if path.components().any(|c| c.as_os_str() == ".git")
        || manifest.excluded.iter().any(|e| path.starts_with(e))
    {
        return Ok(());
    }
    let Some(meta) = inspect(&manifest.root, selection)? else {
        return if allow_missing {
            Ok(())
        } else {
            Err(
                Error::new("FILE_NOT_FOUND", 5, "A selected source is missing.")
                    .details(serde_json::json!({"path": selection})),
            )
        };
    };
    if meta.is_file() {
        if files.contains_key(selection) {
            return Ok(());
        }
        if files.len() == MAX_FILES {
            return Err(Error::new(
                "SIZE_LIMIT",
                9,
                "Source manifest exceeds 10000 files; select narrower paths.",
            ));
        }
        let entry = hash_file(&manifest.root, selection)?.ok_or_else(|| {
            Error::new(
                "SOURCE_CHANGED",
                4,
                "A selected source disappeared during capture.",
            )
        })?;
        files.insert(selection.into(), entry);
    } else if meta.is_dir() {
        let mut children = fs::read_dir(manifest.root.join(selection))
            .map_err(|e| Error::io("Read source directory", e))?
            .map(|e| e.map(|e| e.path()))
            .collect::<std::io::Result<Vec<_>>>()
            .map_err(|e| Error::io("Read source entry", e))?;
        children.sort();
        for child in children {
            let relative = native_text(
                child
                    .strip_prefix(&manifest.root)
                    .map_err(|_| Error::invalid("Source escaped root."))?,
            )?;
            // Normalize leading ./ so overlapping selections identify each file only once.
            let relative = native_text(&normalize(Path::new(&relative)))?;
            scan(manifest, &relative, files, allow_missing)?;
        }
    } else {
        return Err(Error::new(
            "UNSAFE_SOURCE",
            4,
            "Only regular source files and real directories may be selected.",
        ));
    }
    Ok(())
}

fn selected_path(path: &Path) -> Result<PathBuf> {
    let mut prefix = PathBuf::new();
    for component in path.components() {
        prefix.push(component.as_os_str());
        if matches!(component, Component::Normal(_)) {
            let meta = fs::symlink_metadata(&prefix)
                .map_err(|e| Error::io("Inspect selected source", e))?;
            if meta.file_type().is_symlink() {
                return Err(Error::new(
                    "UNSAFE_SOURCE",
                    4,
                    "Selected source paths cannot traverse symbolic links.",
                ));
            }
            #[cfg(windows)]
            {
                use std::os::windows::fs::MetadataExt;
                if meta.file_attributes() & 0x400 != 0 {
                    return Err(Error::new(
                        "UNSAFE_SOURCE",
                        4,
                        "Selected source paths cannot traverse reparse points.",
                    ));
                }
            }
        }
    }
    fs::canonicalize(path).map_err(|e| Error::io("Resolve selected source", e))
}

pub fn capture(context: &Context, paths: &[PathBuf]) -> Result<Manifest> {
    if paths.is_empty() {
        return Err(Error::invalid("Select at least one source path."));
    }
    let root = context
        .project_root
        .clone()
        .unwrap_or_else(|| context.cwd.clone());
    native_text(&root)?;
    let mut manifest = Manifest {
        schema: 1,
        root,
        selections: Vec::new(),
        excluded: Vec::new(),
        files: BTreeMap::new(),
    };
    if let Ok(store) = normalize(&context.store_base).strip_prefix(&manifest.root) {
        let path = native_text(store)?;
        if !path.is_empty() {
            manifest.excluded.push(path);
        } else {
            return Err(Error::invalid(
                "The source root cannot also be the storage base.",
            ));
        }
    }
    for path in paths {
        let absolute = selected_path(&context.cwd.join(path))?;
        let relative = absolute.strip_prefix(&manifest.root).map_err(|_| {
            Error::invalid("Snapshot selections must be inside the selected source root.")
        })?;
        let relative = native_text(relative)?;
        manifest.selections.push(if relative.is_empty() {
            ".".into()
        } else {
            relative
        });
    }
    manifest.selections.sort();
    manifest.selections.dedup();
    let mut files = BTreeMap::new();
    for selection in &manifest.selections {
        scan(&manifest, selection, &mut files, false)?;
    }
    manifest.files = files;
    if manifest.files.is_empty() {
        return Err(Error::new(
            "EMPTY_SELECTION",
            8,
            "No source files were selected.",
        ));
    }
    Ok(manifest)
}

pub fn save(store: &Store, manifest: &Manifest, max_entries: usize) -> Result<Metadata> {
    let mut staged = store.staging_file()?;
    serde_json::to_writer(&mut staged, manifest)
        .map_err(|e| Error::new("IO_ERROR", 5, e.to_string()))?;
    staged.flush().map_err(|e| Error::io("Flush manifest", e))?;
    store.put_record(
        staged.as_file_mut(),
        RecordKind::Snapshot,
        Vec::new(),
        max_entries,
    )
}

pub fn load(store: &Store, id: &str) -> Result<Manifest> {
    let (metadata, bytes) = store.get(id)?;
    if metadata.kind != RecordKind::Snapshot {
        return Err(Error::new(
            "WRONG_KIND",
            4,
            "Expected a source snapshot reference.",
        ));
    }
    let manifest: Manifest = serde_json::from_slice(&bytes)
        .map_err(|_| Error::new("CORRUPT_REF", 7, "Invalid source manifest."))?;
    let valid_relative = |value: &str| {
        !value.is_empty()
            && !Path::new(value).is_absolute()
            && Path::new(value)
                .components()
                .all(|c| matches!(c, Component::Normal(_) | Component::CurDir))
    };
    let invalid_files = manifest.files.iter().any(|(path, entry)| {
        !valid_relative(path)
            || path == "."
            || !manifest
                .selections
                .iter()
                .any(|selection| selection == "." || Path::new(path).starts_with(selection))
            || entry.sha256.len() != 64
            || !entry
                .sha256
                .bytes()
                .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
    });
    if manifest.schema != 1
        || !manifest.root.is_absolute()
        || manifest.files.is_empty()
        || manifest.files.len() > MAX_FILES
        || manifest.selections.is_empty()
        || manifest.selections.iter().any(|p| !valid_relative(p))
        || manifest.excluded.iter().any(|p| !valid_relative(p))
        || invalid_files
    {
        return Err(Error::new(
            "CORRUPT_REF",
            7,
            "Invalid source manifest schema or root.",
        ));
    }
    Ok(manifest)
}

pub fn changes(before: &Manifest, after: &Manifest) -> Result<Vec<Change>> {
    if before.root != after.root {
        return Err(Error::invalid("Snapshots have different source roots."));
    }
    let mut changes = Vec::new();
    for (path, old) in &before.files {
        match after.files.get(path) {
            None => changes.push(Change {
                path: path.clone(),
                status: "removed",
            }),
            Some(new) if old != new => changes.push(Change {
                path: path.clone(),
                status: "modified",
            }),
            _ => {}
        }
    }
    for path in after.files.keys() {
        if !before.files.contains_key(path) {
            changes.push(Change {
                path: path.clone(),
                status: "added",
            });
        }
    }
    changes.sort_by(|a, b| a.path.cmp(&b.path));
    Ok(changes)
}

pub fn check(manifest: &Manifest) -> Result<Vec<Change>> {
    match fs::symlink_metadata(&manifest.root) {
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            return Ok(manifest
                .files
                .keys()
                .map(|path| Change {
                    path: path.clone(),
                    status: "removed",
                })
                .collect())
        }
        Err(e) => return Err(Error::io("Inspect source root", e)),
        Ok(meta) if !meta.is_dir() || meta.file_type().is_symlink() => {
            return Err(Error::new(
                "UNSAFE_SOURCE",
                4,
                "Source root is no longer a real directory.",
            ))
        }
        Ok(_) => {}
    }
    let mut current = manifest.clone();
    current.files.clear();
    for selection in &manifest.selections {
        scan(manifest, selection, &mut current.files, true)?;
    }
    changes(manifest, &current)
}
