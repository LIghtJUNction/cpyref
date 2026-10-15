# Real-model engineering pilot protocol

Frozen on 2026-10-15 before observing model outcomes. This experiment may produce a negative result. It is not designed to manufacture a favorable claim.

## Question and scope

Does adding CpyRef 0.3.0 to an otherwise identical shell-based agent improve task acceptance, backend token use, or elapsed time on six executable maintenance fixtures?

The fixtures are constructed software maintenance tasks, not sampled GitHub issues and not SWE-bench. They contain working code, files, and executable acceptance checks, but cannot represent software engineering as a whole. Four tasks involve exact-byte content reuse; two test diagnosis and logic changes. This selection is deliberately favorable to the proposed use case and must not be reported as a representative industry sample.

Test CpyRef by installing frozen latest main `0c869fdc31e21490a6a7cbfa90f9e0229c46af04` from Git. Version 0.3.0 was already released; the main revision and executable digest are recorded separately from the release tag. Do not substitute the old benchmark binary.

## Conditions

- Native: POSIX shell, Python, cp, sed, grep, and pipelines. The prompt explicitly encourages programmatic copying instead of retyping long data.
- CpyRef available: identical native tools, plus the installed CpyRef binary and concise command guidance. The model may decline to use it. Count actual adoption rather than assuming availability means use.

The CpyRef guidance is included in actual model input and therefore charged in token totals. There is no weakened baseline that forbids efficient shell commands. No file contents are prefilled into model-generated patches by the experiment code.

## Actual model and budget

Use `bartowski/Qwen_Qwen3-4B-Instruct-2507-GGUF`, file `Qwen_Qwen3-4B-Instruct-2507-Q4_K_M.gguf`, a quantization of Qwen/Qwen3-4B-Instruct-2507. Resolve the public repository to an immutable revision and verify the model file SHA-256. Use an official llama.cpp Ubuntu CPU release, record its release tag and SHA-256, and require all shards to match. No paid API and no external coding agent are used.

Each task-condition pair gets seed 17, temperature 0.2, top_p 0.95, maximum 5 model turns and 768 generated tokens per turn. The final done message consumes a turn. Invalid JSON, unsuccessful commands, and output-limit failures consume budget. Do not selectively repeat bad outcomes. Inference-server or missing-usage failures make the pilot incomplete rather than favorable.

Use a JSON action interface. Each response chooses a shell command or done. The local backend actually generates every action. Save complete requests, responses, backend usage, and tool results. Do not replace real generations with expected commands.

Six tasks times two conditions produce 12 episodes. Three parallel runners each handle two paired tasks sequentially. Task order is shuffled with seed 17; condition order alternates across the shuffled order. This is a single-seed exploratory pilot, not a confidence interval or non-inferiority test.

## Isolation and acceptance

Generated commands execute in fresh Docker containers. No network, host home, API credentials, GitHub token, Docker socket, model runtime, benchmark source, golden solution, or hidden oracle is mounted. Only the generated task workspace is writable. Treatment also gets a read-only CpyRef binary. Containers have a read-only root, no extra capabilities, a non-root user, memory/CPU/process limits, and per-command timeouts.

Both arms get the same 2048-byte-per-stream head/tail display limit and a 20-second command limit. Full command output remains in experiment evidence. Agents can redirect and inspect logs inside their task workspace. There is no hidden larger output allowance for one condition.

Acceptance code lives outside the agent workspace and is supplied only after the episode. Check protected source/test files, requested exact target bytes where exact copying is the task, and behavioral assertions. Before model execution, prove every initial fixture fails and every golden solution passes. For the two logic/diagnosis tasks, accept behaviorally correct alternative implementations, not only one textual patch.

## Predeclared tasks

1. `extract_table`: insert a 96-entry lookup table from legacy.py into catalog.py, preserving source and unrelated bytes.
2. `port_parser`: port the exact existing duration parser into a compatibility module and preserve behavior.
3. `sync_config`: insert one shared configuration block into two targets, preserving their distinct settings.
4. `fixture_copy`: backfill three exact JSON fixture copies; whole-file copying is a native-tool control.
5. `noisy_retry_bug`: fix an off-by-one error using a test that emits verbose output.
6. `labels_logic`: implement ordered case-insensitive deduplication; no-copy control.

## Outcomes

Report acceptance first, then separate input tokens, output tokens, model/tool calls, and wall time. Use actual backend usage, not text length divided by four. Input totals include full conversation requests and treatment instructions. Record token counts per call so totals can be independently recomputed. Cache reuse is disabled in requests; this does not model provider-specific cache billing.

All attempts, including failures, remain in totals. Cost per accepted task uses total resources across all attempts divided by the number accepted. If no tasks pass, that metric is undefined, not zero. Report paired task results so totals cannot hide a regression. CpyRef adoption counts shell turns containing a CpyRef invocation, not exact underlying process counts.

Elapsed episode time includes actual CPU inference and tool execution; acceptance time is measured separately and included in aggregate comparison. Container setup, model download/loading, and binary compilation are excluded from episode latency and retained in workflow evidence. Do not compare CPU timings to cloud-model latency.

No API fees are billed by the local inference server. Compute monetary cost is unknown, not zero. Calculate optional input/output price-ratio sensitivity at output weight 1, 4, and 10, explicitly in token-weight units rather than currency. These are assumptions, not measured savings on a commercial invoice. Do not transplant this model's tokenizer or speed into cloud-model pricing claims.

## Evidence and interpretation

Retain every task manifest, tested commit, source digest, model/runtime identity, raw response usage, generated command, acceptance result, and failed attempt. Publish an audit that requires all 12 unique episodes, identical runtimes, and exact trace-to-total accounting. Keep infrastructure failures in their original run directories.

A lower total with worse acceptance is not enough to claim improved efficiency. A favorable result is evidence only for this model, budget, and selected task set. It does not prove gains for stronger models, larger repositories, arbitrary bug fixes, commercial bills, or day-to-day software engineering. An unfavorable result must be reported with the same prominence.

Sources: https://github.com/LIghtJUNction/cpyref ; https://huggingface.co/Qwen/Qwen3-4B-Instruct-2507 ; https://huggingface.co/bartowski/Qwen_Qwen3-4B-Instruct-2507-GGUF ; https://github.com/ggml-org/llama.cpp/tree/master/tools/server .
