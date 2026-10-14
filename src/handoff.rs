use crate::{
    context::{Context, Options, Scope},
    error::{Error, Result},
    snapshot::native_text,
    store::{Metadata, RecordKind},
};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::path::{Path, PathBuf};

const PREFIX: &str = "cpyref:v1:";

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Ticket {
    pub reference: String,
    pub sha256: String,
    pub kind: RecordKind,
    pub scope: Scope,
    pub namespace: String,
    pub store: PathBuf,
    pub project: Option<PathBuf>,
    pub cwd: PathBuf,
}

pub fn token(context: &Context, metadata: &Metadata) -> Result<String> {
    // Refuse lossy path serialization, just as shell handoffs do.
    native_text(&context.store_base)?;
    native_text(&context.cwd)?;
    if let Some(path) = &context.project_root {
        native_text(path)?;
    }
    let ticket = Ticket {
        reference: metadata.id.clone(),
        sha256: metadata.sha256.clone(),
        kind: metadata.kind,
        scope: context.scope,
        namespace: context.namespace.clone(),
        store: context.store_base.clone(),
        project: context.project_root.clone(),
        cwd: context.cwd.clone(),
    };
    Ok(format!(
        "{PREFIX}{}",
        serde_json::to_string(&ticket).map_err(|e| Error::invalid(e.to_string()))?
    ))
}

pub fn receive(value: &str) -> Result<(Context, Metadata)> {
    if value.len() > 64 * 1024 {
        return Err(Error::invalid("Handoff exceeds 64 KiB."));
    }
    let value = value
        .strip_prefix(PREFIX)
        .ok_or_else(|| Error::invalid("Expected a cpyref:v1: handoff reference."))?;
    let ticket: Ticket =
        serde_json::from_str(value).map_err(|_| Error::invalid("Invalid handoff reference."))?;
    if !ticket.store.is_absolute()
        || !ticket.cwd.is_absolute()
        || ticket.project.as_ref().is_some_and(|p| !p.is_absolute())
        || (ticket.scope == Scope::Project) != ticket.project.is_some()
        || ticket.sha256.len() != 64
        || !ticket
            .sha256
            .bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
    {
        return Err(Error::invalid("Invalid handoff paths, project or SHA-256."));
    }
    // Only directory scope derives its identity from the sender's cwd.
    let current = std::env::current_dir().map_err(|e| Error::io("Read current directory", e))?;
    let cwd = if ticket.scope == Scope::Directory {
        &ticket.cwd
    } else {
        &current
    };
    let context = Context::resolve_at(
        &Options {
            no_config: true,
            store: Some(ticket.store),
            scope: Some(ticket.scope),
            namespace: Some(ticket.namespace),
            project: ticket.project,
            ..Options::default()
        },
        cwd,
        None,
    )?;
    let (_, metadata) = context
        .store()?
        .verified_file(&ticket.reference)
        .map_err(|e| context.annotate(e))?;
    if metadata.sha256 != ticket.sha256 || metadata.kind != ticket.kind {
        return Err(Error::new(
            "HASH_MISMATCH",
            7,
            "Saved record does not match the handoff hash or kind.",
        ));
    }
    Ok((context, metadata))
}

pub fn handoff(context: &Context, id: &str, format: &str, executable: &Path) -> Result<Value> {
    let (_, metadata) = context.store()?.verified_file(id)?;
    let token = token(context, &metadata)?;
    let scope = match context.scope {
        Scope::Project => "project",
        Scope::Directory => "directory",
        Scope::Shared => "shared",
        Scope::Global => "global",
    };
    let mut argv = vec![
        native_text(executable)?,
        "--no-config".into(),
        "--store".into(),
        native_text(&context.store_base)?,
        "--scope".into(),
        scope.into(),
        "--namespace".into(),
        context.namespace.clone(),
        "--max-entries".into(),
        context.max_entries.to_string(),
    ];
    if let Some(root) = &context.project_root {
        argv.extend(["--project".into(), native_text(root)?]);
    }
    argv.extend(["receive".into(), token.clone()]);
    let command = match format {
        "sh" => argv
            .iter()
            .map(|a| format!("'{}'", a.replace('\'', "'\\''")))
            .collect::<Vec<_>>()
            .join(" "),
        "powershell" => format!(
            "& {}",
            argv.iter()
                .map(|a| format!("'{}'", a.replace('\'', "''")))
                .collect::<Vec<_>>()
                .join(" ")
        ),
        "json" | "ref" => String::new(),
        _ => {
            return Err(Error::invalid(
                "Handoff format must be json, ref, sh or powershell.",
            ))
        }
    };
    // Remove inherited CPYREF_* variables: --no-config otherwise conflicts with CPYREF_CONFIG.
    // Directory scope additionally needs the exact working directory.
    let cwd = native_text(&context.cwd)?;
    let rendered = if format == "sh" {
        format!(
            "( unset CPYREF_CONFIG CPYREF_PROJECT; cd -- '{}' && {} )",
            cwd.replace('\'', "'\\''"),
            command
        )
    } else if format == "powershell" {
        format!("& {{ $savedConfig = $env:CPYREF_CONFIG; $savedProject = $env:CPYREF_PROJECT; $savedCwd = Get-Location; try {{ Remove-Item Env:CPYREF_CONFIG,Env:CPYREF_PROJECT -ErrorAction SilentlyContinue; Set-Location -LiteralPath '{}' -ErrorAction Stop; {} }} finally {{ Set-Location -LiteralPath $savedCwd; $env:CPYREF_CONFIG = $savedConfig; $env:CPYREF_PROJECT = $savedProject }} }}", cwd.replace('\'', "''"), command)
    } else {
        command
    };
    Ok(
        json!({"reference": id, "sha256": metadata.sha256, "token": token, "kind": metadata.kind, "context": context.report(), "cwd": context.cwd,
        "argv": argv, "unset_env": ["CPYREF_CONFIG", "CPYREF_PROJECT"], "command": if format == "json" { None } else { Some(rendered) },
        "transport": "same_host", "format": format}),
    )
}
