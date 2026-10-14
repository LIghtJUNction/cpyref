//! Immutable named references; payloads stay in their original records.
use crate::{
    context::Context,
    error::{Error, Result},
    store::{Metadata, RecordKind, Store},
};
use serde::{Deserialize, Serialize};
use std::{collections::BTreeMap, io::Write};

const MAX_ITEMS: usize = 100;
const MAX_MANIFEST: usize = 64 * 1024;

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Item {
    pub reference: String,
    pub sha256: String,
    pub kind: RecordKind,
    pub bytes: usize,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Manifest {
    pub version: u8,
    pub items: BTreeMap<String, Item>,
}

fn valid_name(name: &str) -> bool {
    !name.is_empty()
        && name.len() <= 64
        && name.as_bytes()[0].is_ascii_alphanumeric()
        && name
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b"_.-".contains(&b))
}

pub fn create(context: &Context, items: &[String]) -> Result<Metadata> {
    if items.is_empty() || items.len() > MAX_ITEMS {
        return Err(Error::invalid("A bundle requires 1-100 NAME=REF items."));
    }
    let store = context.store()?;
    let mut manifest = Manifest {
        version: 1,
        items: BTreeMap::new(),
    };
    for item in items {
        let (name, id) = item
            .split_once('=')
            .ok_or_else(|| Error::invalid("Expected NAME=REF."))?;
        if !valid_name(name) || manifest.items.contains_key(name) {
            return Err(Error::invalid("Item names must be unique, 1-64 ASCII letters/digits/_.-, starting with a letter or digit."));
        }
        let (_, metadata) = store.verified_file(id)?;
        manifest.items.insert(
            name.into(),
            Item {
                reference: metadata.id,
                sha256: metadata.sha256,
                kind: metadata.kind,
                bytes: metadata.bytes,
            },
        );
    }
    let data = serde_json::to_vec(&manifest).map_err(|e| Error::invalid(e.to_string()))?;
    if data.len() > MAX_MANIFEST {
        return Err(Error::invalid("Bundle manifest exceeds 64 KiB."));
    }
    let mut dependencies: Vec<_> = manifest
        .items
        .values()
        .map(|i| i.reference.clone())
        .collect();
    dependencies.sort();
    dependencies.dedup();
    let mut staged = store.staging_file()?;
    staged
        .write_all(&data)
        .map_err(|e| Error::io("Write bundle manifest", e))?;
    staged
        .flush()
        .map_err(|e| Error::io("Flush bundle manifest", e))?;
    store.put_record(
        staged.as_file_mut(),
        RecordKind::Bundle,
        dependencies,
        context.max_entries,
    )
}

pub fn load(store: &Store, id: &str) -> Result<Manifest> {
    let metadata = store.info(id)?;
    if metadata.kind != RecordKind::Bundle {
        return Err(Error::new("WRONG_KIND", 4, "Expected a bundle reference."));
    }
    if metadata.bytes > MAX_MANIFEST {
        return Err(Error::new(
            "CORRUPT_REF",
            7,
            "Bundle manifest exceeds 64 KiB.",
        ));
    }
    let (_, data) = store.get(id)?;
    let manifest: Manifest = serde_json::from_slice(&data)
        .map_err(|_| Error::new("CORRUPT_REF", 7, "Invalid bundle manifest."))?;
    let mut dependencies: Vec<_> = manifest
        .items
        .values()
        .map(|i| i.reference.clone())
        .collect();
    dependencies.sort();
    dependencies.dedup();
    if manifest.version != 1
        || manifest.items.is_empty()
        || manifest.items.len() > MAX_ITEMS
        || manifest.items.keys().any(|name| !valid_name(name))
        || dependencies != metadata.dependencies
    {
        return Err(Error::new(
            "CORRUPT_REF",
            7,
            "Invalid bundle items or dependencies.",
        ));
    }
    Ok(manifest)
}

pub fn item(store: &Store, id: &str, name: &str) -> Result<Metadata> {
    let manifest = load(store, id)?;
    let item = manifest
        .items
        .get(name)
        .ok_or_else(|| Error::new("ITEM_NOT_FOUND", 6, "No such item in this bundle."))?;
    let (_, metadata) = store.verified_file(&item.reference)?;
    if metadata.sha256 != item.sha256 || metadata.kind != item.kind || metadata.bytes != item.bytes
    {
        return Err(Error::new(
            "HASH_MISMATCH",
            7,
            "Bundle item no longer matches its pinned record.",
        ));
    }
    Ok(metadata)
}
