use crate::{
    error::{Error, Result},
    select::Span,
    MAX_BYTES,
};
use rand::RngCore;
use serde::{Deserialize, Serialize};
use serde_json::json;
use sha2::{Digest, Sha256};
use std::{
    fs::{self, File, OpenOptions},
    io::{Read, Seek, SeekFrom, Write},
    path::{Component, Path, PathBuf},
    time::{SystemTime, UNIX_EPOCH},
};
use tempfile::NamedTempFile;

const MAGIC: &[u8; 8] = b"CPYREF\x01\n";
const MAX_HEADER: usize = 64 * 1024;

#[derive(Debug, Default, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, clap::ValueEnum)]
#[serde(rename_all = "snake_case")]
pub enum RecordKind {
    #[default]
    Bytes,
    Snapshot,
    Run,
    Bundle,
    EditPlan,
}

impl RecordKind {
    fn is_bytes(&self) -> bool {
        *self == Self::Bytes
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Metadata {
    pub version: u8,
    #[serde(default, skip_serializing_if = "RecordKind::is_bytes")]
    pub kind: RecordKind,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub dependencies: Vec<String>,
    pub id: String,
    pub bytes: usize,
    pub sha256: String,
    pub created_at: u64,
    pub source: String,
    pub span: Span,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub note: Option<String>,
}

#[derive(Debug, Clone)]
pub struct Store {
    base: PathBuf,
    root: PathBuf,
}

fn digest(data: &[u8]) -> String {
    format!("{:x}", Sha256::digest(data))
}

fn corrupt(message: &str) -> Error {
    Error::new("CORRUPT_REF", 7, message)
}

pub(crate) fn valid_id(id: &str) -> bool {
    id.len() == 13
        && id.starts_with('c')
        && id.as_bytes()[1..]
            .iter()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(b))
}

fn missing_reference() -> Error {
    Error::new(
        "REF_NOT_FOUND",
        6,
        "The reference does not exist in this store.",
    )
}

fn unsafe_store() -> Error {
    Error::new("UNSAFE_STORE", 5, "The storage base and all managed subdirectories must be real, private directories, not files or symbolic links.")
}

fn check_directory(path: &Path, allow_missing: bool) -> Result<()> {
    let metadata = match fs::symlink_metadata(path) {
        Ok(metadata) => metadata,
        Err(e) if allow_missing && e.kind() == std::io::ErrorKind::NotFound => return Ok(()),
        Err(e) => return Err(Error::io("Open store directory", e)),
    };
    if metadata.file_type().is_symlink() || !metadata.is_dir() {
        return Err(unsafe_store());
    }
    #[cfg(windows)]
    {
        use std::os::windows::fs::MetadataExt;
        // Reject junctions and other reparse points within the managed store tree as well.
        if metadata.file_attributes() & 0x400 != 0 {
            return Err(unsafe_store());
        }
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        if metadata.permissions().mode() & 0o077 != 0 {
            return Err(Error::new("UNSAFE_STORE", 5, "The store must not be accessible by group or other users; use private directories with mode 0700."));
        }
    }
    Ok(())
}

impl Store {
    /// Low-level exact directory API. CLI callers use Context for project isolation.
    pub fn new(root: PathBuf) -> Self {
        Self {
            base: root.clone(),
            root,
        }
    }

    pub fn scoped(base: PathBuf, root: PathBuf) -> Result<Self> {
        let relative = root.strip_prefix(&base).map_err(|_| unsafe_store())?;
        if relative
            .components()
            .any(|part| !matches!(part, Component::Normal(_)))
        {
            return Err(unsafe_store());
        }
        Ok(Self { base, root })
    }

    pub fn default_root() -> Result<PathBuf> {
        dirs::data_local_dir()
            .map(|p| p.join("cpyref"))
            .ok_or_else(|| {
                Error::new(
                    "STORE_UNAVAILABLE",
                    5,
                    "Cannot locate the user data directory; set --store or CPYREF_STORE.",
                )
            })
    }

    fn check_root(&self, allow_missing: bool) -> Result<()> {
        let relative = self
            .root
            .strip_prefix(&self.base)
            .map_err(|_| unsafe_store())?;
        let mut path = self.base.clone();
        check_directory(&path, allow_missing)?;
        for component in relative.components() {
            path.push(component);
            check_directory(&path, allow_missing)?;
        }
        Ok(())
    }

    fn prepare(&self) -> Result<()> {
        // Check existing ancestors before recursive creation so a symlink cannot redirect a space.
        self.check_root(true)?;
        let mut builder = fs::DirBuilder::new();
        builder.recursive(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::DirBuilderExt;
            builder.mode(0o700);
        }
        builder.create(&self.root).map_err(|e| {
            if matches!(
                e.kind(),
                std::io::ErrorKind::AlreadyExists | std::io::ErrorKind::NotADirectory
            ) {
                unsafe_store()
            } else {
                Error::io("Create store directory", e)
            }
        })?;
        self.check_root(false)
    }

    fn path(&self, id: &str) -> Result<PathBuf> {
        if !valid_id(id) {
            return Err(Error::new(
                "INVALID_REF",
                4,
                "Use the complete reference: c followed by 12 lowercase hex digits.",
            ));
        }
        self.check_root(false).map_err(|error| {
            if error.code == "FILE_NOT_FOUND" {
                missing_reference()
            } else {
                error
            }
        })?;
        Ok(self.root.join(format!("{id}.ref")))
    }

    /// Publish one complete record without replacing another reference, even across processes.
    pub fn put(&self, data: &[u8], source: String, span: Span) -> Result<Metadata> {
        self.put_with_options(data, source, span, None, 0)
    }

    /// All cooperating writers serialize count checks and publication in this space.
    fn lock(&self) -> Result<File> {
        self.check_root(false)?;
        let path = self.root.join(".write.lock");
        match fs::symlink_metadata(&path) {
            Ok(meta) if !meta.is_file() || meta.file_type().is_symlink() => {
                return Err(unsafe_store())
            }
            Ok(_) => {}
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
            Err(e) => return Err(Error::io("Inspect store lock", e)),
        }
        let mut options = OpenOptions::new();
        options.read(true).write(true).create(true).truncate(false);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            options.mode(0o600);
        }
        let file = options
            .open(path)
            .map_err(|e| Error::io("Open store lock", e))?;
        file.lock().map_err(|e| Error::io("Lock store", e))?;
        Ok(file)
    }

    pub(crate) fn edit_lock(&self) -> Result<File> {
        self.prepare()?;
        self.lock()
    }

    pub fn put_with_options(
        &self,
        data: &[u8],
        source: String,
        span: Span,
        note: Option<String>,
        max_entries: usize,
    ) -> Result<Metadata> {
        if data.is_empty() {
            return Err(Error::new(
                "EMPTY_SELECTION",
                8,
                "The selected content is empty.",
            ));
        }
        if data.len() > MAX_BYTES || span.byte_end.checked_sub(span.byte_start) != Some(data.len())
        {
            return Err(Error::invalid("Snapshot length and selected byte range do not agree, or exceed the hard size limit."));
        }
        if note.as_ref().is_some_and(|n| n.len() > 4096) {
            return Err(Error::invalid(
                "Notes may contain at most 4096 UTF-8 bytes.",
            ));
        }
        let metadata = Self::metadata(
            data.len(),
            digest(data),
            source,
            span,
            note,
            RecordKind::Bytes,
            Vec::new(),
        )?;
        self.publish(&mut std::io::Cursor::new(data), metadata, max_entries)
    }

    fn metadata(
        bytes: usize,
        sha256: String,
        source: String,
        span: Span,
        note: Option<String>,
        kind: RecordKind,
        dependencies: Vec<String>,
    ) -> Result<Metadata> {
        Ok(Metadata {
            version: 1,
            kind,
            dependencies,
            id: String::new(),
            bytes,
            sha256,
            created_at: SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .map_err(|_| {
                    Error::new(
                        "CLOCK_ERROR",
                        5,
                        "The system clock is before the Unix epoch.",
                    )
                })?
                .as_secs(),
            source,
            span,
            note,
        })
    }

    /// Stream typed records from a private staged file; run logs have no in-memory size ceiling.
    pub fn put_record(
        &self,
        file: &mut File,
        kind: RecordKind,
        dependencies: Vec<String>,
        max_entries: usize,
    ) -> Result<Metadata> {
        if kind == RecordKind::Bytes {
            return Err(Error::invalid("Use copy for byte snapshots."));
        }
        let bytes = usize::try_from(
            file.metadata()
                .map_err(|e| Error::io("Read staged size", e))?
                .len(),
        )
        .map_err(|_| {
            Error::new(
                "SIZE_LIMIT",
                9,
                "Record cannot be represented on this platform.",
            )
        })?;
        if bytes == 0 || (kind != RecordKind::Run && bytes > MAX_BYTES) {
            return Err(Error::new("SIZE_LIMIT", 9, "Invalid typed record size."));
        }
        file.rewind()
            .map_err(|e| Error::io("Rewind staged record", e))?;
        let sha256 = hash_reader(file)?;
        let selector = match kind {
            RecordKind::Run => "run",
            RecordKind::Bundle => "bundle",
            RecordKind::EditPlan => "edit_plan",
            _ => "snapshot",
        };
        let span = Span {
            byte_start: 0,
            byte_end: bytes,
            line_start: 1,
            line_end: 1,
            selector: selector.into(),
        };
        let metadata = Self::metadata(
            bytes,
            sha256,
            format!("<{selector}>"),
            span,
            None,
            kind,
            dependencies,
        )?;
        self.publish(file, metadata, max_entries)
    }

    pub fn available_space(&self) -> Result<u64> {
        self.prepare()?;
        fs2::available_space(&self.root).map_err(|e| Error::io("Check store free space", e))
    }

    pub fn staging_file(&self) -> Result<NamedTempFile> {
        self.prepare()?;
        NamedTempFile::new_in(&self.root).map_err(|e| Error::io("Create staged record", e))
    }

    pub fn check_capacity(&self, max_entries: usize) -> Result<()> {
        self.prepare()?;
        let _lock = self.lock()?;
        self.capacity(max_entries)
    }

    fn capacity(&self, max_entries: usize) -> Result<()> {
        if max_entries != 0 {
            let count = self.ids()?.len();
            if count >= max_entries {
                return Err(Error::new("ENTRY_LIMIT", 9, "This space has reached max_entries; preview cleanup or raise the configured limit.")
                    .details(json!({"count": count, "max_entries": max_entries})));
            }
        }
        Ok(())
    }

    fn publish(
        &self,
        reader: &mut (impl Read + Seek),
        mut metadata: Metadata,
        max_entries: usize,
    ) -> Result<Metadata> {
        self.prepare()?;
        let _lock = self.lock()?;
        self.capacity(max_entries)?;
        for dependency in &metadata.dependencies {
            self.info(dependency)?;
        }
        for _ in 0..32 {
            let mut random = [0_u8; 6];
            rand::rngs::OsRng.try_fill_bytes(&mut random).map_err(|e| {
                Error::new("RANDOM_ERROR", 5, format!("Cannot create a reference: {e}"))
            })?;
            metadata.id = format!(
                "c{}",
                random
                    .iter()
                    .map(|b| format!("{b:02x}"))
                    .collect::<String>()
            );
            let header =
                serde_json::to_vec(&metadata).map_err(|e| Error::invalid(e.to_string()))?;
            if header.len() > MAX_HEADER {
                return Err(Error::new(
                    "SIZE_LIMIT",
                    9,
                    "Snapshot metadata exceeds 64 KiB.",
                ));
            }
            let mut temp =
                NamedTempFile::new_in(&self.root).map_err(|e| Error::io("Create snapshot", e))?;
            temp.write_all(MAGIC)
                .and_then(|_| temp.write_all(&(header.len() as u32).to_le_bytes()))
                .and_then(|_| temp.write_all(&header))
                .and_then(|_| reader.rewind())
                .and_then(|_| std::io::copy(reader, &mut temp).map(|_| ()))
                .and_then(|_| temp.as_file().sync_all())
                .map_err(|e| Error::io("Write snapshot", e))?;
            let actual_len = temp
                .as_file()
                .metadata()
                .map_err(|e| Error::io("Read staged length", e))?
                .len();
            temp.seek(SeekFrom::Start(12 + header.len() as u64))
                .map_err(|e| Error::io("Seek staged payload", e))?;
            if actual_len != 12 + header.len() as u64 + metadata.bytes as u64
                || hash_reader(temp.as_file_mut())? != metadata.sha256
            {
                return Err(Error::new(
                    "SOURCE_CHANGED",
                    4,
                    "Staged bytes changed before publication; no reference was published.",
                ));
            }
            let destination = self.root.join(format!("{}.ref", metadata.id));
            match temp.persist_noclobber(destination) {
                Ok(_) => return Ok(metadata),
                Err(e) if e.error.kind() == std::io::ErrorKind::AlreadyExists => continue,
                Err(e) => return Err(Error::io("Publish snapshot", e.error)),
            }
        }
        Err(Error::new(
            "ID_COLLISION",
            5,
            "Could not allocate a unique reference after 32 attempts.",
        ))
    }

    fn open(&self, id: &str) -> Result<(File, Metadata)> {
        // Validate before touching the filesystem, and never resolve shortened prefixes.
        let path = self.path(id)?;
        let metadata = fs::symlink_metadata(&path).map_err(|e| {
            if e.kind() == std::io::ErrorKind::NotFound {
                missing_reference()
            } else {
                Error::io("Read reference", e)
            }
        })?;
        if !metadata.is_file() || metadata.file_type().is_symlink() {
            return Err(corrupt(
                "The reference must be a regular file, not a symbolic link.",
            ));
        }
        let mut file = File::open(path).map_err(|e| Error::io("Open snapshot", e))?;
        let file_length = file
            .metadata()
            .map_err(|e| Error::io("Read snapshot size", e))?
            .len();
        let mut prefix = [0_u8; 12];
        file.read_exact(&mut prefix)
            .map_err(|_| corrupt("Snapshot header is incomplete."))?;
        if &prefix[..8] != MAGIC {
            return Err(corrupt("Unknown snapshot format."));
        }
        let header_length =
            u32::from_le_bytes([prefix[8], prefix[9], prefix[10], prefix[11]]) as usize;
        if header_length == 0 || header_length > MAX_HEADER {
            return Err(corrupt("Snapshot header length is invalid."));
        }
        let mut header = vec![0; header_length];
        file.read_exact(&mut header)
            .map_err(|_| corrupt("Snapshot metadata is incomplete."))?;
        let metadata: Metadata = serde_json::from_slice(&header)
            .map_err(|_| corrupt("Snapshot metadata is invalid."))?;
        if metadata.version != 1
            || metadata.id != id
            || metadata.bytes == 0
            || (metadata.kind != RecordKind::Run && metadata.bytes > MAX_BYTES)
            || metadata.dependencies.iter().any(|id| !valid_id(id))
            || metadata.span.byte_end.checked_sub(metadata.span.byte_start) != Some(metadata.bytes)
            || metadata.sha256.len() != 64
            || (12 + header_length as u64).checked_add(metadata.bytes as u64) != Some(file_length)
        {
            return Err(corrupt(
                "Snapshot metadata or payload length is inconsistent.",
            ));
        }
        Ok((file, metadata))
    }

    /// Reads metadata only. Payload integrity is checked by `get`, not by `info`.
    pub fn info(&self, id: &str) -> Result<Metadata> {
        self.open(id).map(|(_, metadata)| metadata)
    }

    /// Verify the complete payload before making any of its bytes available to the caller.
    pub fn get(&self, id: &str) -> Result<(Metadata, Vec<u8>)> {
        let (file, metadata) = self.open(id)?;
        if metadata.bytes > MAX_BYTES {
            return Err(Error::new(
                "SIZE_LIMIT",
                9,
                "Use bounded logs or a streaming export for this record.",
            ));
        }
        let mut data = Vec::with_capacity(metadata.bytes);
        file.take(metadata.bytes as u64 + 1)
            .read_to_end(&mut data)
            .map_err(|e| Error::io("Read snapshot payload", e))?;
        if data.len() != metadata.bytes || digest(&data) != metadata.sha256 {
            return Err(corrupt(
                "Snapshot payload does not match its recorded length and SHA-256 hash.",
            ));
        }
        Ok((metadata, data))
    }

    /// Search and hash the entire saved payload in one pass, using a bounded overlap window.
    pub fn contains_payload(
        &self,
        id: &str,
        matcher: &regex::bytes::Regex,
        overlap: usize,
    ) -> Result<bool> {
        let (mut file, metadata) = self.open(id)?;
        let mut digest = Sha256::new();
        let mut buffer = [0_u8; 65536];
        let mut tail = Vec::new();
        let mut window = Vec::new();
        let mut found = false;
        let mut bytes = 0_u64;
        loop {
            let n = file
                .read(&mut buffer)
                .map_err(|e| Error::io("Search saved payload", e))?;
            if n == 0 {
                break;
            }
            digest.update(&buffer[..n]);
            bytes += n as u64;
            if !found {
                window.clear();
                window.extend_from_slice(&tail);
                window.extend_from_slice(&buffer[..n]);
                found = matcher.is_match(&window);
                let keep = window.len().min(overlap);
                tail.clear();
                tail.extend_from_slice(&window[window.len() - keep..]);
            }
        }
        if bytes != metadata.bytes as u64 || format!("{:x}", digest.finalize()) != metadata.sha256 {
            return Err(
                corrupt("Saved payload failed integrity verification during search.")
                    .details(json!({"id": id})),
            );
        }
        Ok(found)
    }

    /// Verify a full payload before exposing a seekable stream, using bounded memory.
    pub fn verified_file(&self, id: &str) -> Result<(File, Metadata)> {
        let (mut file, metadata) = self.open(id)?;
        let offset = file
            .stream_position()
            .map_err(|e| Error::io("Read payload offset", e))?;
        if hash_reader(&mut file)? != metadata.sha256 {
            return Err(corrupt("Snapshot payload does not match SHA-256."));
        }
        file.seek(SeekFrom::Start(offset))
            .map_err(|e| Error::io("Rewind payload", e))?;
        Ok((file, metadata))
    }

    /// No retained record may lose a dependency; callers can remove dependents in the same batch.
    pub fn check_removal(&self, selected: &[Metadata]) -> Result<()> {
        self.check_removal_ids(&selected.iter().map(|m| m.id.as_str()).collect::<Vec<_>>())
    }

    fn check_removal_ids(&self, selected: &[&str]) -> Result<()> {
        for id in self.ids()? {
            if selected.contains(&id.as_str()) {
                continue;
            }
            let m = match self.info(&id) {
                Ok(m) => m,
                // An unreadable header has no usable dependency declaration. Exact remove
                // must still permit recovery when more than one record is damaged.
                Err(error) if error.code == "CORRUPT_REF" => continue,
                Err(error) => return Err(error),
            };
            if let Some(dependency) = m
                .dependencies
                .iter()
                .find(|d| selected.contains(&d.as_str()))
            {
                return Err(Error::new(
                    "REF_IN_USE",
                    4,
                    "A retained record depends on a selected reference.",
                )
                .details(json!({"reference": dependency, "used_by": id})));
            }
        }
        Ok(())
    }

    fn ids(&self) -> Result<Vec<String>> {
        self.check_root(true)?;
        let entries = match fs::read_dir(&self.root) {
            Ok(entries) => entries,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
            Err(e) => return Err(Error::io("List space", e)),
        };
        let mut ids = Vec::new();
        for entry in entries {
            let entry = entry.map_err(|e| Error::io("List space entry", e))?;
            let name = entry.file_name();
            if let Some(id) = name.to_str().and_then(|n| n.strip_suffix(".ref")) {
                if valid_id(id) {
                    ids.push(id.to_owned());
                }
            }
        }
        Ok(ids)
    }

    /// Metadata only, sorted newest first with an ID tie breaker. Missing space is empty.
    /// Corrupt records fail closed rather than silently disappearing from inventory.
    pub fn list(&self) -> Result<Vec<Metadata>> {
        let mut entries = self
            .ids()?
            .iter()
            .map(|id| self.info(id).map_err(|e| e.details(json!({"id": id}))))
            .collect::<Result<Vec<_>>>()?;
        entries.sort_by(|a, b| {
            b.created_at
                .cmp(&a.created_at)
                .then_with(|| a.id.cmp(&b.id))
        });
        Ok(entries)
    }

    pub fn stored_bytes(&self, id: &str) -> Result<u64> {
        let (file, _) = self.open(id)?;
        file.metadata()
            .map(|m| m.len())
            .map_err(|e| Error::io("Read stored size", e))
    }

    /// Revalidate every planned record under the writer lock before the first deletion.
    pub fn remove_batch(&self, expected: &[Metadata]) -> Result<()> {
        if expected.is_empty() {
            return Ok(());
        }
        let _lock = self.lock()?;
        self.check_removal(expected)?;
        for metadata in expected {
            let current = self.info(&metadata.id)?;
            if serde_json::to_vec(&current).ok() != serde_json::to_vec(metadata).ok() {
                return Err(Error::new(
                    "PLAN_CHANGED",
                    4,
                    "A planned reference changed; preview cleanup again.",
                ));
            }
        }
        let mut pending = expected.iter().collect::<Vec<_>>();
        let mut ordered = Vec::new();
        while !pending.is_empty() {
            let Some(index) = pending.iter().position(|m| {
                !pending
                    .iter()
                    .any(|other| other.dependencies.contains(&m.id))
            }) else {
                return Err(corrupt("Reference dependency cycle."));
            };
            ordered.push(pending.remove(index));
        }
        let mut removed = Vec::new();
        for metadata in ordered {
            self.remove_unlocked(&metadata.id).map_err(|error| {
                error.details(json!({"removed": removed, "failed": metadata.id}))
            })?;
            removed.push(metadata.id.clone());
        }
        Ok(())
    }

    pub fn remove(&self, id: &str) -> Result<()> {
        self.path(id)?;
        let _lock = self.lock()?;
        self.check_removal_ids(&[id])?;
        self.remove_unlocked(id)
    }

    fn remove_unlocked(&self, id: &str) -> Result<()> {
        // A damaged record can still be removed explicitly by its exact validated ID.
        let path = self.path(id)?;
        fs::remove_file(path).map_err(|e| {
            if e.kind() == std::io::ErrorKind::NotFound {
                missing_reference()
            } else {
                Error::io("Remove snapshot", e)
            }
        })
    }
}

pub fn hash_reader(reader: &mut impl Read) -> Result<String> {
    let mut hash = Sha256::new();
    let mut buffer = [0_u8; 65536];
    loop {
        let n = reader
            .read(&mut buffer)
            .map_err(|e| Error::io("Hash bytes", e))?;
        if n == 0 {
            break;
        }
        hash.update(&buffer[..n]);
    }
    Ok(format!("{:x}", hash.finalize()))
}

/// Write through a temporary file in the destination directory, never truncate in place.
pub fn write_output(path: &Path, data: &[u8], force: bool) -> Result<()> {
    write_stream_output(path, &mut std::io::Cursor::new(data), force)
}

pub fn write_stream_output(path: &Path, reader: &mut impl Read, force: bool) -> Result<()> {
    if let Ok(metadata) = fs::symlink_metadata(path) {
        if metadata.file_type().is_symlink() || !metadata.is_file() {
            return Err(Error::new(
                "UNSAFE_OUTPUT",
                5,
                "The output must not be a symbolic link or a non-regular file.",
            ));
        }
        if !force {
            return Err(Error::new(
                "OUTPUT_EXISTS",
                5,
                "Output already exists; use --force to replace it.",
            ));
        }
    }
    let parent = path
        .parent()
        .filter(|p| !p.as_os_str().is_empty())
        .unwrap_or_else(|| Path::new("."));
    let mut temp = NamedTempFile::new_in(parent).map_err(|e| Error::io("Create output file", e))?;
    std::io::copy(reader, &mut temp)
        .and_then(|_| temp.as_file().sync_all())
        .map_err(|e| Error::io("Write output", e))?;
    let result = if force {
        temp.persist(path)
    } else {
        temp.persist_noclobber(path)
    };
    result
        .map(|_| ())
        .map_err(|e| Error::io("Publish output file", e.error))
}
