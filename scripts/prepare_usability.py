"""Apply reviewed, exact source edits to the pinned parent; removed before delivery."""
from pathlib import Path


def replace(path, old, new, count=1):
    p = Path(path)
    text = p.read_text()
    actual = text.count(old)
    if actual != count:
        raise SystemExit(f"{path}: expected {count} matches, found {actual}: {old[:100]!r}")
    p.write_text(text.replace(old, new))

replace('src/store.rs', '    Bundle,\n}', '    Bundle,\n    EditPlan,\n}')
replace('src/store.rs', 'fn valid_id(id: &str) -> bool {', 'pub(crate) fn valid_id(id: &str) -> bool {')
replace('src/store.rs', '            RecordKind::Bundle => "bundle",', '            RecordKind::Bundle => "bundle",\n            RecordKind::EditPlan => "edit_plan",')
replace('src/search.rs', '        RecordKind::Bundle => "bundle",', '        RecordKind::Bundle => "bundle",\n        RecordKind::EditPlan => "edit_plan",')
replace('src/lib.rs', 'pub mod edit;\n', 'pub mod edit;\npub mod plan;\npub mod reference;\n')

replace('src/edit.rs', '#[derive(Debug, ClapArgs)]\npub struct Args {', '#[derive(Debug, Clone, ClapArgs, serde::Serialize, serde::Deserialize)]\n#[serde(deny_unknown_fields)]\npub struct Args {')
replace('src/edit.rs', '    /// Required for writing: target SHA-256 from the approved preview or read.', '''    /// Save a guarded edit plan without changing the target; use cpyref apply PLAN_ID next.
    #[arg(long, conflicts_with_all = ["dry_run", "expect_sha256"])]
    pub plan: bool,
    /// Required for direct writing: target SHA-256 from the approved preview or read.''')
replace('src/edit.rs', 'fn validate(args: &Args, replace: bool) -> Result<()> {', '''fn validate(args: &Args, replace: bool) -> Result<()> {
    if args.plan && (args.dry_run || args.expect_sha256.is_some()) {
        return Err(Error::invalid("Use --plan by itself, or the existing --dry-run / --expect-sha256 route."));
    }''')
replace('src/edit.rs', '    } else if !args.dry_run {', '    } else if !args.dry_run && !args.plan {')
replace('src/edit.rs', '"Preview with --dry-run, then supply its before_sha256 using --expect-sha256."', '"Use --plan, then cpyref apply PLAN_ID. For direct writes, preview with --dry-run and provide --expect-sha256."')
replace('src/edit.rs', 'pub fn execute(context: &Context, args: &Args, replace: bool) -> Result<Value> {\n    validate(args, replace)?;', '''pub fn execute(context: &Context, args: &Args, replace: bool) -> Result<Value> {
    execute_checked(context, args, replace, None)
}

pub(crate) fn execute_checked(context: &Context, args: &Args, replace: bool, expected_source: Option<&str>) -> Result<Value> {
    validate(args, replace)?;''')
replace('src/edit.rs', '    let requested = context.cwd.join(&args.target);', '''    if expected_source.is_some_and(|expected| expected != reference.sha256.as_str()) {
        return Err(Error::new("REF_CHANGED", 10,
            "The saved bytes no longer match this plan. Nothing was written; create a new plan from trusted input."));
    }
    let requested = context.cwd.join(&args.target);''')
replace('src/edit.rs', '    let _lock = if args.dry_run {', '    let _lock = if args.dry_run || args.plan {')
replace('src/edit.rs', '    if !args.dry_run {', '    if !args.dry_run && !args.plan {')
replace('src/edit.rs', '''    Ok(
        json!({"ok": true, "reference": args.id, "target": path, "operation": if replace { "replace" } else { "insert" },
        "applied": !args.dry_run, "byte_start": range.start, "byte_end": range.end,
        "bytes_removed": range.len(), "bytes_inserted": inserted.len(), "output_bytes": after.len(),
        "before_sha256": before_sha256, "after_sha256": after_sha256, "context": context.report()}),
    )''', '''    let report = json!({"ok": true, "reference": args.id, "target": path, "operation": if replace { "replace" } else { "insert" },
        "applied": !args.dry_run && !args.plan, "byte_start": range.start, "byte_end": range.end,
        "bytes_removed": range.len(), "bytes_inserted": inserted.len(), "output_bytes": after.len(),
        "before_sha256": before_sha256, "after_sha256": after_sha256, "context": context.report()});
    if args.plan {
        crate::plan::save(context, args, replace, &reference.sha256, &report)
    } else {
        Ok(report)
    }''')

replace('src/cli.rs', '    Replace(Box<cpyref::edit::Args>),', '''    Replace(Box<cpyref::edit::Args>),
    /// Apply a saved edit plan; its source and target hashes are checked automatically.
    #[command(display_order = 5)]
    Apply {
        /// Complete plan ID, or - to read the ID/plan JSON from a pipe.
        id: String,
        /// Recheck the plan and preview it without writing.
        #[arg(long)]
        dry_run: bool,
    },''')
for variant, order in [('Copy(Box<CopyArgs>),', 1), ('Paste {', 2), ('Insert(Box<cpyref::edit::Args>),', 3), ('Replace(Box<cpyref::edit::Args>),', 4), ('Info { id: String },', 6), ('Context,', 7)]:
    replace('src/cli.rs', '    ' + variant, f'    #[command(display_order = {order})]\n    ' + variant)
replace('src/cli.rs', '    about = "Copy exact bytes once. Paste them by a short reference."', '''    about = "Copy exact bytes once. Paste them by a short reference.",
    after_help = "Examples:\\n  cpyref copy source.txt | cpyref paste - --output destination.txt\\n  cpyref copy source.rs --lines 10:20 | cpyref insert - target.rs --at-end --plan\\n  cpyref apply PLAN_ID  (use the returned plan ID; keep the same space)"''')

replace('src/main.rs', '        Command::Insert(args) => print_json', '''        Command::Apply { id, dry_run } => print_json(&cpyref::plan::apply(context, &id, dry_run)?),
        Command::Insert(args) => print_json''')
replace('src/main.rs', 'if store.info(&id)?.kind == cpyref::store::RecordKind::Run {', 'if matches!(store.info(&id)?.kind, cpyref::store::RecordKind::Run | cpyref::store::RecordKind::EditPlan) {')
replace('src/main.rs', '"Use logs REF --stream stdout/stderr to retrieve a run\'s logs."', '"Use cpyref logs REF --stream stdout/stderr for logs, or cpyref apply PLAN_ID --dry-run for an edit plan."')
replace('src/main.rs', 'fn run(cli: Cli) -> Result<u8> {', 'fn run(mut cli: Cli) -> Result<u8> {')
replace('src/main.rs', '    let context = Context::resolve(&cli.context)?;\n    let result = match cli.command {', '''    let context = Context::resolve(&cli.context)?;
    match &mut cli.command {
        Command::Apply { id, .. } => *id = cpyref::reference::resolve(&context, id, true)?,
        Command::Insert(args) | Command::Replace(args) => {
            args.id = cpyref::reference::resolve(&context, &args.id, false)?;
        }
        Command::Paste { id, .. } | Command::Info { id } | Command::Remove { id }
        | Command::Verify { id } | Command::Handoff { id, .. } | Command::Logs { id, .. } => {
            *id = cpyref::reference::resolve(&context, id, false)?;
        }
        _ => {}
    }
    let result = match cli.command {''')
replace('src/workflows.rs', '            RecordKind::Bytes => unreachable!(),', '''            RecordKind::EditPlan => print_json(&cpyref::plan::show(context, &metadata.id)?)?,
            RecordKind::Bytes => unreachable!(),''')

replace('README.md', '''# 复制成功后，将实际编号直接传给 paste。
ref=$(cpyref copy input.txt --lines 20:40) &&
    cpyref paste "$ref" --output snippet.rs''', '''# 管道只传编号；正文不会经过模型。
cpyref copy input.txt --lines 20:40 | cpyref paste - --output snippet.rs''')
replace('README.md', '## 空间与安全', '''## 少传编号，不抄哈希

以下新流程需从包含 `apply` 的源码安装，旧发布包可能没有：

```sh
set -o pipefail  # Bash；保留上游失败状态
cpyref copy source.rs --lines 10:20 | cpyref insert - target.rs --at-end --plan
# 检查返回的目标、位置和大小，在相同空间运行 next_command。
cpyref apply PLAN_ID
```

`PLAN_ID` 用实际返回的 `plan` 替换。第一条命令不修改目标；第二条自动核对计划中的来源和目标哈希。目标已变化就停止，不会自动刷新哈希。替换用 `cpyref replace ... --plan`。原有显式哈希方式保留；完整规则见[管道与编辑计划](docs/easy-edits.md)。

## 空间与安全''')
replace('README.md', '`insert`、`replace`；先预览，再提供目标哈希；[安全写入](docs/hardening.md)', '`cpyref insert/replace ... --plan`、`cpyref apply PLAN_ID`；[编辑计划](docs/easy-edits.md)；[底层写入](docs/hardening.md)')
replace('docs/agent-contract.md', '## 失败处理', '''## 简单编辑流程

`cpyref copy FILE | cpyref insert - TARGET --at-end --plan` 只生成计划；检查目标和范围后，在相同空间运行返回的 `cpyref apply PLAN_ID`。无需提取哈希。替换使用 `cpyref replace`。不得因 `TARGET_CHANGED` 自动重新生成计划后重试。新流程需更新源码，详见[管道与编辑计划](easy-edits.md)。

命令示例必须带 `cpyref` 前缀。`-` 传入一个完整编号或成功的对应 JSON，不传正文。源选择器不可混用；需要给标记限定范围时使用 `--within`，而不是叠加 `--lines`。

## 失败处理''')
