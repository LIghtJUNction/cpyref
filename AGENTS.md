# Contributing

CpyRef stores exact-byte snapshots in project-scoped local spaces. Before changes, read `README.md`, `docs/agent-contract.md`, `LICENSE`, and `UPSTREAM.toml`. Preserve byte fidelity, isolation, stored data, and failure reporting.

Run `cargo fmt --all -- --check`, `cargo test --locked --all-targets`, and `cargo clippy --locked --all-targets -- -D warnings`. Report failures and checks not run. Keep examples short; link to the relevant reference instead of repeating it.

This original repository is its own upstream. Do not create self-addressed issues or PRs solely for license compliance. A downstream fork following an external upstream must report confirmed upstream defects and offer retained improvements through an accepted channel. Check for existing submissions. Submit only authorized, safe material; if blocked, report the reason, recipient, and pending work. Never claim an unsubmitted draft was delivered.

Never publish clipboard contents, private snippets, secrets, or customer data. Follow secure disclosure. Do not change upstream or elect independence without maintainer authorization; copying `UPSTREAM.toml` does not make a new fork independent. License and contribution details remain in `LICENSE`; historical MIT grants are unchanged.
