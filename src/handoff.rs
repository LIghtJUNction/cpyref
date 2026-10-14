use crate::{
    context::{Context, Scope},
    error::{Error, Result},
    snapshot::native_text,
    store::RecordKind,
};
use serde_json::{json, Value};
use std::path::Path;

pub fn handoff(context: &Context, id: &str, format: &str, executable: &Path) -> Result<Value> {
    let metadata = context.store()?.info(id)?;
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
    match metadata.kind {
        RecordKind::Run => argv.extend(["logs".into(), id.into()]),
        RecordKind::Snapshot => argv.extend(["snapshot".into(), "show".into(), id.into()]),
        RecordKind::Bytes => argv.extend(["paste".into(), id.into()]),
    }
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
        "json" => String::new(),
        _ => {
            return Err(Error::invalid(
                "Handoff format must be json, sh or powershell.",
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
        json!({"reference": id, "kind": metadata.kind, "context": context.report(), "cwd": context.cwd,
        "argv": argv, "unset_env": ["CPYREF_CONFIG", "CPYREF_PROJECT"], "command": if format == "json" { None } else { Some(rendered) },
        "transport": "same_host", "format": format}),
    )
}
