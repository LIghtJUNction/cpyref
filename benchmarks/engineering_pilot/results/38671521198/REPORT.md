# CpyRef engineering pilot — measured results

**Scope:** six constructed, executable maintenance tasks. These are not mined real-world issues.
Real local-model decisions, commands, responses, and backend token usage are retained. No completions are scripted.
Model: Qwen3-4B-Instruct-2507, Q4_K_M. One seed (17), five-turn maximum. This is a pilot, not general proof.

Complete-data audit: **FAIL** (0/12 episodes).

| Condition | Passed | Input tokens | Output tokens | Time incl. acceptance (s) | Tool calls | Used CpyRef |
|---|---:|---:|---:|---:|---:|---:|
| native | 0/0 | 0 | 0 | 0.00 | 0 | 0 |
| cpyref | 0/0 | 0 | 0 | 0.00 | 0 | 0 |

All failed attempts remain in token/time totals. Tokens per success also charge failed attempts.

| Task | Native pass | CpyRef pass | Native input/output | CpyRef input/output | Native/CpyRef seconds |
|---|---|---|---:|---:|---:|

## Interpretation rules

- Treat success, token use, and elapsed time as separate outcomes. Cheap failures are not efficiency gains.
- The native condition retains Python, cp, sed, grep, and pipes. CpyRef is optional in the treatment.
- Treatment-specific help is included in every actual model request and its token count.
- Latency includes local CPU inference and Docker command execution, not model download/build/startup.
- API fees are zero for this local run. Compute is not free by definition; no electricity/cloud price was measured.
- The 1:1, 1:4, and 1:10 input/output price-ratio cases are sensitivity calculations, not observed invoices.
- No cloud-model pricing or tokenizer can be inferred from this one quantized local model.
- Six convenience-selected tasks and one seed cannot establish broad software-engineering superiority.
- No unsuccessful model response is silently rerun or excluded. Infrastructure problems invalidate completeness.

## Audit errors

- Missing or duplicate predeclared episodes
- Inconsistent or missing environment: model_sha256
- Inconsistent or missing environment: model_revision
- Inconsistent or missing environment: llama_tag
- Inconsistent or missing environment: llama_archive_sha256
- Inconsistent or missing environment: docker_image
