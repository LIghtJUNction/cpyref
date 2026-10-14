use crate::{error::{Error, Result}, select::Span, MAX_BYTES};
use rand::RngCore;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{fs::{self, File}, io::{Read, Write}, path::{Component, Path, PathBuf}, time::{SystemTime, UNIX_EPOCH}};
use tempfile::NamedTempFile;

const MAGIC: &[u8; 8] = b"CPYREF\x01\n";
const MAX_HEADER: usize = 64 * 1024;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Metadata {
    pub version: u8,
    pub id: String,
    pub bytes: usize,
    pub sha256: String,
    pub created_at: u64,
    pub source: String,
    pub span: Span,
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

fn valid_id(id: &str) -> bool {
    id.len() == 13 && id.starts_with('c') && id.as_bytes()[1..].iter().all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(b))
}

fn missing_reference() -> Error {
    Error::new("REF_NOT_FOUND", 6, "The reference does not exist in this store.")
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
        Self { base: root.clone(), root }
    }

    pub fn scoped(base: PathBuf, root: PathBuf) -> Result<Self> {
        let relative = root.strip_prefix(&base).map_err(|_| unsafe_store())?;
        if relative.components().any(|part| !matches!(part, Component::Normal(_))) {
            return Err(unsafe_store());
        }
        Ok(Self { base, root })
    }

    pub fn default_root() -> Result<PathBuf> {
        dirs::data_local_dir().map(|p| p.join("cpyref")).ok_or_else(|| {
            Error::new("STORE_UNAVAILABLE", 5, "Cannot locate the user data directory; set --store or CPYREF_STORE.")
        })
    }

    fn check_root(&self, allow_missing: bool) -> Result<()> {
        let relative = self.root.strip_prefix(&self.base).map_err(|_| unsafe_store())?;
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
            if matches!(e.kind(), std::io::ErrorKind::AlreadyExists | std::io::ErrorKind::NotADirectory) {
                unsafe_store()
            } else {
                Error::io("Create store directory", e)
            }
        })?;
        self.check_root(false)
    }

    fn path(&self, id: &str) -> Result<PathBuf> {
        if !valid_id(id) {
            return Err(Error::new("INVALID_REF", 4, "Use the complete reference: c followed by 12 lowercase hex digits."));
        }
        self.check_root(false).map_err(|error| {
            if error.code == "FILE_NOT_FOUND" { missing_reference() } else { error }
        })?;
        Ok(self.root.join(format!("{id}.ref")))
    }

    /// Publish one complete record without replacing another reference, even across processes.
    pub fn put(&self, data: &[u8], source: String, span: Span) -> Result<Metadata> {
        if data.is_empty() {
            return Err(Error::new("EMPTY_SELECTION", 8, "The selected content is empty."));
        }
        if data.len() > MAX_BYTES || span.byte_end.checked_sub(span.byte_start) != Some(data.len()) {
            return Err(Error::invalid("Snapshot length and selected byte range do not agree, or exceed the hard size limit."));
        }
        self.prepare()?;
        let mut metadata = Metadata {
            version: 1,
            id: String::new(),
            bytes: data.len(),
            sha256: digest(data),
            created_at: SystemTime::now().duration_since(UNIX_EPOCH)
                .map_err(|_| Error::new("CLOCK_ERROR", 5, "The system clock is before the Unix epoch."))?.as_secs(),
            source,
            span,
        };
        for _ in 0..32 {
            let mut random = [0_u8; 6];
            rand::rngs::OsRng.try_fill_bytes(&mut random)
                .map_err(|e| Error::new("RANDOM_ERROR", 5, format!("Cannot create a reference: {e}")))?;
            metadata.id = format!("c{}", random.iter().map(|b| format!("{b:02x}")).collect::<String>());
            let header = serde_json::to_vec(&metadata).map_err(|e| Error::invalid(e.to_string()))?;
            if header.len() > MAX_HEADER {
                return Err(Error::new("SIZE_LIMIT", 9, "Snapshot metadata exceeds 64 KiB."));
            }
            let mut temp = NamedTempFile::new_in(&self.root).map_err(|e| Error::io("Create snapshot", e))?;
            temp.write_all(MAGIC)
                .and_then(|_| temp.write_all(&(header.len() as u32).to_le_bytes()))
                .and_then(|_| temp.write_all(&header))
                .and_then(|_| temp.write_all(data))
                .and_then(|_| temp.as_file().sync_all())
                .map_err(|e| Error::io("Write snapshot", e))?;
            let destination = self.root.join(format!("{}.ref", metadata.id));
            match temp.persist_noclobber(destination) {
                Ok(_) => return Ok(metadata),
                Err(e) if e.error.kind() == std::io::ErrorKind::AlreadyExists => continue,
                Err(e) => return Err(Error::io("Publish snapshot", e.error)),
            }
        }
        Err(Error::new("ID_COLLISION", 5, "Could not allocate a unique reference after 32 attempts."))
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
            return Err(corrupt("The reference must be a regular file, not a symbolic link."));
        }
        let mut file = File::open(path).map_err(|e| Error::io("Open snapshot", e))?;
        let file_length = file.metadata().map_err(|e| Error::io("Read snapshot size", e))?.len();
        let mut prefix = [0_u8; 12];
        file.read_exact(&mut prefix).map_err(|_| corrupt("Snapshot header is incomplete."))?;
        if &prefix[..8] != MAGIC {
            return Err(corrupt("Unknown snapshot format."));
        }
        let header_length = u32::from_le_bytes([prefix[8], prefix[9], prefix[10], prefix[11]]) as usize;
        if header_length == 0 || header_length > MAX_HEADER {
            return Err(corrupt("Snapshot header length is invalid."));
        }
        let mut header = vec![0; header_length];
        file.read_exact(&mut header).map_err(|_| corrupt("Snapshot metadata is incomplete."))?;
        let metadata: Metadata = serde_json::from_slice(&header).map_err(|_| corrupt("Snapshot metadata is invalid."))?;
        if metadata.version != 1 || metadata.id != id || metadata.bytes == 0 || metadata.bytes > MAX_BYTES
            || metadata.span.byte_end.checked_sub(metadata.span.byte_start) != Some(metadata.bytes)
            || metadata.sha256.len() != 64
            || file_length != 12 + header_length as u64 + metadata.bytes as u64
        {
            return Err(corrupt("Snapshot metadata or payload length is inconsistent."));
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
        let mut data = Vec::with_capacity(metadata.bytes);
        file.take(metadata.bytes as u64 + 1).read_to_end(&mut data)
            .map_err(|e| Error::io("Read snapshot payload", e))?;
        if data.len() != metadata.bytes || digest(&data) != metadata.sha256 {
            return Err(corrupt("Snapshot payload does not match its recorded length and SHA-256 hash."));
        }
        Ok((metadata, data))
    }

    pub fn remove(&self, id: &str) -> Result<()> {
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

/// Write through a temporary file in the destination directory, never truncate in place.
pub fn write_output(path: &Path, data: &[u8], force: bool) -> Result<()> {
    if let Ok(metadata) = fs::symlink_metadata(path) {
        if metadata.file_type().is_symlink() || !metadata.is_file() {
            return Err(Error::new("UNSAFE_OUTPUT", 5, "The output must not be a symbolic link or a non-regular file."));
        }
        if !force {
            return Err(Error::new("OUTPUT_EXISTS", 5, "Output already exists; use --force to replace it."));
        }
    }
    let parent = path.parent().filter(|p| !p.as_os_str().is_empty()).unwrap_or_else(|| Path::new("."));
    let mut temp = NamedTempFile::new_in(parent).map_err(|e| Error::io("Create output file", e))?;
    temp.write_all(data).and_then(|_| temp.as_file().sync_all()).map_err(|e| Error::io("Write output", e))?;
    let result = if force { temp.persist(path) } else { temp.persist_noclobber(path) };
    result.map(|_| ()).map_err(|e| Error::io("Publish output file", e.error))
}
