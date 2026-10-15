//! Immutable, project-scoped edit plans. Loading or receiving a plan never writes its target.
use crate::{
    context::Context,
    edit::{self, Args},
    error::{Error, Result},
    snapshot::native_text,
    store::{valid_id, RecordKind},
};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::path::PathBuf;

const MAX_PLAN: usize = 64 * 1024;

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Plan {
    schema: u8,
    store_path: PathBuf,
    args: Args,
    replace: bool,
    source_sha256: String,
    preview: Value,
}

fn corrupt() -> Error {
    Error::new(
        "CORRUPT_REF",
        7,
        "Invalid edit plan. Create a new plan from trusted inputs.",
    )
}

fn hash(value: &str) -> bool {
    value.len() == 64 && value.bytes().all(|b| b.is_ascii_hexdigit())
}

pub fn save(
    context: &Context,
    args: &Args,
    replace: bool,
    source_sha256: &str,
    preview: &Value,
) -> Result<Value> {
    let mut args = args.clone();
    args.target = context.cwd.join(&args.target);
    native_text(&args.target)?;
    native_text(&context.store_path)?;
    args.expect_sha256 = Some(
        preview["before_sha256"]
            .as_str()
            .ok_or_else(corrupt)?
            .to_owned(),
    );
    args.plan = false;
    args.dry_run = false;
    let plan = Plan {
        schema: 1,
        store_path: context.store_path.clone(),
        args,
        replace,
        source_sha256: source_sha256.to_owned(),
        preview: preview.clone(),
    };
    let bytes = serde_json::to_vec(&plan).map_err(|_| corrupt())?;
    if bytes.len() > MAX_PLAN {
        return Err(Error::new("SIZE_LIMIT", 9,
            "Edit plan exceeds 64 KiB. Use a shorter selector or the existing --expect-sha256 workflow."));
    }
    let store = context.store()?;
    let mut staged = store.staging_file()?;
    std::io::Write::write_all(&mut staged, &bytes).map_err(|e| Error::io("Write edit plan", e))?;
    let metadata = store.put_record(
        staged.as_file_mut(),
        RecordKind::EditPlan,
        vec![plan.args.id.clone()],
        context.max_entries,
    )?;
    Ok(report(&metadata.id, &plan))
}

fn load(context: &Context, id: &str) -> Result<Plan> {
    let store = context.store()?;
    let metadata = store.info(id)?;
    if metadata.kind != RecordKind::EditPlan {
        return Err(Error::new("WRONG_KIND", 4,
            "cpyref apply requires an edit plan. Create one with cpyref insert/replace REF FILE ... --plan."));
    }
    if metadata.bytes > MAX_PLAN {
        return Err(corrupt());
    }
    let (metadata, bytes) = store.get(id)?;
    let plan: Plan = serde_json::from_slice(&bytes).map_err(|_| corrupt())?;
    if metadata.kind != RecordKind::EditPlan
        || plan.schema != 1
        || plan.store_path != context.store_path
        || !plan.args.target.is_absolute()
        || !valid_id(&plan.args.id)
        || plan.args.plan
        || plan.args.dry_run
        || !plan.args.expect_sha256.as_deref().is_some_and(hash)
        || !hash(&plan.source_sha256)
        || !plan.preview.is_object()
        || metadata.dependencies != [plan.args.id.clone()]
    {
        return Err(corrupt());
    }
    Ok(plan)
}

fn report(id: &str, plan: &Plan) -> Value {
    let mut report = plan.preview.clone();
    report["plan"] = json!(id);
    report["kind"] = json!("edit_plan");
    report["applied"] = json!(false);
    report["status"] = json!("planned");
    report["next_command"] = json!(format!("cpyref apply {id}"));
    report
}

pub fn show(context: &Context, id: &str) -> Result<Value> {
    Ok(report(id, &load(context, id)?))
}

pub fn apply(context: &Context, id: &str, dry_run: bool) -> Result<Value> {
    let mut plan = load(context, id)?;
    plan.args.dry_run = dry_run;
    // Reuse the same editor and lock. Never refresh a stale hash or reinterpret the selection.
    let mut result =
        edit::execute_checked(context, &plan.args, plan.replace, Some(&plan.source_sha256))?;
    result["plan"] = json!(id);
    result["status"] = json!(if dry_run { "preview" } else { "applied" });
    Ok(result)
}
