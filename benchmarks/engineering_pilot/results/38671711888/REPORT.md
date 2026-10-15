# CpyRef engineering pilot — measured results

**Scope:** six constructed, executable maintenance tasks. These are not mined real-world issues.
Real local-model decisions, commands, responses, and backend token usage are retained. No completions are scripted.
Model: Qwen3-4B-Instruct-2507, Q4_K_M. One seed (17), five-turn maximum. This is a pilot, not general proof.

Complete-data audit: **FAIL** (12/12 episodes).

| Condition | Passed | Input tokens | Output tokens | Time incl. acceptance (s) | Tool calls | Used CpyRef |
|---|---:|---:|---:|---:|---:|---:|
| native | 0/6 | 17887 | 1715 | 1259.48 | 21 | 0 |
| cpyref | 1/6 | 33476 | 1737 | 1775.74 | 26 | 2 |

All failed attempts remain in token/time totals. Tokens per success also charge failed attempts.

| Task | Native pass | CpyRef pass | Native input/output | CpyRef input/output | Native/CpyRef seconds |
|---|---|---|---:|---:|---:|
| extract_table | False | False | 2022/24 | 529/12 | 267.20/202.25 |
| port_parser | False | False | 3953/904 | 5697/536 | 256.12/298.29 |
| sync_config | False | False | 1918/27 | 9657/144 | 261.90/436.37 |
| fixture_copy | False | False | 5040/169 | 8299/98 | 225.22/364.13 |
| noisy_retry_bug | False | False | 1885/100 | 4364/127 | 81.50/185.49 |
| labels_logic | False | True | 3069/491 | 4930/820 | 167.53/289.21 |

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

- Incomplete inference: extract_table/native
- Incomplete inference: extract_table/cpyref
- Incomplete inference: sync_config/native
