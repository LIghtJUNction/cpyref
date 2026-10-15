use clap::{Args, Parser, Subcommand};
use cpyref::{context::Options, select::Spec, store::RecordKind, DEFAULT_MAX_BYTES};
use std::path::PathBuf;

#[derive(Debug, Parser)]
#[command(
    version,
    about = "Copy exact bytes once. Paste them by a short reference.",
    before_help = "Agent handoff: notes -> attach key files -> forward the returned retrieval block\n  cpyref handoff --input HANDOFF.md --attach patch=change.patch\n  cat HANDOFF.md | cpyref handoff --input -\n  For pipes, set -o pipefail to preserve producer failures.\n  The next agent lists the package first, then reads only selected items.\n  Same host and accessible original storage required; saved files are historical snapshots.",
    after_help = "Copy exact bytes:\n  cpyref copy source.txt | cpyref paste - --output destination.txt\n\nAgent shorthand: 'cpy' means cpyref; optional POSIX interactive alias: alias cpy=cpyref"
)]
pub struct Cli {
    #[command(flatten)]
    pub context: Options,
    #[command(subcommand)]
    pub command: Command,
}

#[derive(Debug, Subcommand)]
pub enum Command {
    /// Restore a complete handoff reference, ignoring local context settings; verify its hash first.
    #[command(group(clap::ArgGroup::new("receive_view").args(["list", "read"])))]
    Receive {
        token: String,
        /// Select one named item when receiving a bundle.
        #[arg(long)]
        item: Option<String>,
        /// Show a bounded package directory or metadata, never payload bytes.
        #[arg(long, conflicts_with_all = ["item", "output"])]
        list: bool,
        /// Inspect byte references as bounded JSON, including a selected package item.
        #[arg(long, conflicts_with = "output")]
        read: bool,
        /// Directory entry offset with --list; saved byte offset with --read.
        #[arg(long, default_value_t = 0, requires = "receive_view")]
        offset: u64,
        #[arg(long, default_value_t = 20, value_parser = clap::value_parser!(u32).range(1..=100), requires = "list")]
        limit: u32,
        #[arg(long, default_value_t = 4096, value_parser = clap::value_parser!(u32).range(1..=65536), requires = "read")]
        max_bytes: u32,
        #[arg(short, long)]
        output: Option<PathBuf>,
        #[arg(long, requires = "output")]
        force: bool,
    },
    /// Group existing references into an immutable named handoff package.
    Bundle {
        #[command(subcommand)]
        command: BundleCommand,
    },
    /// Insert saved bytes at one explicit target position. Writing requires a target hash.
    #[command(display_order = 3)]
    Insert(Box<cpyref::edit::Args>),
    /// Replace a unique target selection with saved bytes. Writing requires a target hash.
    #[command(display_order = 4)]
    Replace(Box<cpyref::edit::Args>),
    /// Apply a saved edit plan; its source and target hashes are checked automatically.
    #[command(display_order = 5)]
    Apply {
        /// Complete plan ID, or - to read the ID/plan JSON from a pipe.
        id: String,
        /// Recheck the plan and preview it without writing.
        #[arg(long)]
        dry_run: bool,
    },
    /// Run a command, retain complete local logs, and print only a bounded JSON summary.
    Run(RunArgs),
    /// Inspect a saved run; expand logs only with an explicit stream or --failed.
    Logs {
        id: String,
        /// Select execution arguments, tested inputs or observed artifacts instead of the summary.
        #[arg(long, default_value = "summary", value_parser = ["summary", "command", "inputs", "artifacts"], conflicts_with_all = ["failed", "stream"])]
        view: String,
        /// Show bounded failure positions/context; offset counts diagnostic locations, not tests.
        #[arg(long, conflicts_with = "stream")]
        failed: bool,
        /// Compare this run's recorded source and artifact hashes with current files.
        #[arg(long, conflicts_with_all = ["failed", "stream", "view"])]
        check: bool,
        #[arg(long, value_parser = ["stdout", "stderr"])]
        stream: Option<String>,
        #[arg(long, default_value_t = 0)]
        offset: u64,
        #[arg(long, default_value_t = 20, value_parser = clap::value_parser!(u32).range(1..=1000))]
        limit: u32,
        #[arg(long, default_value_t = 4096, value_parser = clap::value_parser!(u32).range(1..=65536))]
        max_bytes: u32,
        /// Show the last --max-bytes bytes of the selected log stream.
        #[arg(long, requires = "stream", conflicts_with_all = ["offset", "failed", "check", "view", "output"])]
        tail: bool,
        /// Export the complete stream as exact bytes, protecting existing output files.
        #[arg(long, requires = "stream", conflicts_with = "failed")]
        output: Option<PathBuf>,
        #[arg(long, requires = "output")]
        force: bool,
    },
    /// Capture selected source hashes, or check/compare/show an existing manifest.
    Snapshot(SnapshotArgs),
    /// Check whether copied bytes still match their original source position.
    Verify { id: String },
    /// Hand notes and key files to the next agent in one step, or forward a saved reference.
    #[command(display_order = 0)]
    Handoff {
        /// Existing reference, or - for a reference pipe. Omit when using --input.
        #[arg(required_unless_present = "input", conflicts_with_all = ["input", "attach", "max_bytes"])]
        id: Option<String>,
        /// Handoff notes file, or - to read raw notes from stdin. Saves a notes item.
        #[arg(long)]
        input: Option<PathBuf>,
        /// Add a key file as NAME=FILE; repeat up to 99 times. The name notes is reserved.
        #[arg(long, requires = "input")]
        attach: Vec<String>,
        /// Combined notes and attachment byte limit (default 16 MiB; hard maximum 128 MiB).
        #[arg(long, default_value_t = DEFAULT_MAX_BYTES, requires = "input")]
        max_bytes: usize,
        /// Default: agent when creating from --input, json when forwarding an existing reference.
        #[arg(long, value_parser = ["agent", "json", "ref", "sh", "powershell"])]
        format: Option<String>,
    },
    /// Save a unique selection; print only its short reference by default.
    #[command(display_order = 1)]
    Copy(Box<CopyArgs>),
    /// Write the saved bytes, without adding a newline.
    #[command(display_order = 2)]
    Paste {
        id: String,
        /// Write to a file instead of stdout. Existing files are protected by default.
        #[arg(short, long)]
        output: Option<PathBuf>,
        /// Replace an existing regular output file.
        #[arg(long, requires = "output")]
        force: bool,
    },
    /// Inspect saved bytes as bounded JSON; use paste --output for exact-byte export.
    Read {
        /// Complete reference, or - to read a reference from a pipe.
        id: String,
        /// Byte offset within the saved reference, not the original source file.
        #[arg(long, default_value_t = 0, conflicts_with = "tail")]
        offset: u64,
        /// Maximum input bytes to display; JSON escaping may increase output size.
        #[arg(long, default_value_t = 4096, value_parser = clap::value_parser!(u32).range(1..=65536))]
        max_bytes: u32,
        /// One-based inclusive lines of the saved reference: N or START:END.
        #[arg(long, conflicts_with = "tail")]
        lines: Option<String>,
        /// Show the last --max-bytes bytes of the saved reference.
        #[arg(long)]
        tail: bool,
    },
    /// Locate literal text inside one saved reference, with bounded context and positions.
    Find {
        /// Complete reference, or - to read a reference from a pipe.
        id: String,
        query: String,
        #[arg(short = 'i', long)]
        ignore_case: bool,
        /// Number of matches to skip, not a byte offset.
        #[arg(long, default_value_t = 0)]
        offset: usize,
        #[arg(long, default_value_t = 20, value_parser = clap::value_parser!(u32).range(1..=1000))]
        limit: u32,
        /// Total input-byte budget for excerpts across all returned matches.
        #[arg(long, default_value_t = 4096, value_parser = clap::value_parser!(u32).range(1..=65536))]
        max_bytes: u32,
        /// Input bytes of context on each side of a match.
        #[arg(long, default_value_t = 80, value_parser = clap::value_parser!(u32).range(0..=4096))]
        context_bytes: u32,
    },
    /// Print JSON metadata without expanding the copied content.
    #[command(display_order = 6)]
    Info { id: String },
    /// Delete one saved snapshot by its complete reference.
    #[command(alias = "rm")]
    Remove { id: String },
    /// Show the selected project, namespace, store, and config files; never creates a store.
    #[command(display_order = 7)]
    Context {
        /// Print only the exact selected storage directory; never creates it.
        #[arg(long)]
        store_path: bool,
    },
    /// List metadata in this space, newest first; never expand payloads.
    List {
        #[command(flatten)]
        filter: FilterArgs,
        #[arg(long, value_enum)]
        kind: Option<RecordKind>,
        #[arg(long, default_value_t = 20, value_parser = clap::value_parser!(u32).range(1..=1000))]
        limit: u32,
        #[arg(long, default_value_t = 0)]
        offset: usize,
        #[arg(long)]
        json: bool,
        /// Print only full reference IDs, one per line, without a header.
        #[arg(long, conflicts_with = "json")]
        ids: bool,
    },
    /// Find saved references by ID, source, note or exact copied text, without expanding bodies.
    Search {
        query: String,
        /// Search only metadata; skip copied payloads for a faster lookup.
        #[arg(long)]
        metadata_only: bool,
        #[arg(short = 'i', long)]
        ignore_case: bool,
        #[arg(long, value_enum)]
        kind: Option<RecordKind>,
        #[command(flatten)]
        filter: FilterArgs,
        #[arg(long, default_value_t = 20, value_parser = clap::value_parser!(u32).range(1..=1000))]
        limit: u32,
        #[arg(long, default_value_t = 0)]
        offset: usize,
        #[arg(long)]
        json: bool,
        /// Print only matching full reference IDs, one per line, without a header.
        #[arg(long, conflicts_with = "json")]
        ids: bool,
    },
    /// Show counts, sizes and age buckets for this space without reading payloads.
    Stats,
    /// Preview a batch deletion. Apply only with the exact preview confirmation token.
    Clean {
        /// Complete reference IDs; cannot be combined with filters or --all.
        ids: Vec<String>,
        #[command(flatten)]
        filter: FilterArgs,
        /// Select all entries (required if no IDs or filters are given).
        #[arg(long)]
        all: bool,
        /// Select entries older than this many days (86400 seconds each).
        #[arg(long)]
        older_than: Option<u32>,
        /// Protect this many newest entries in the entire selected space.
        #[arg(long)]
        keep_latest: Option<usize>,
        #[arg(long, requires = "confirm")]
        apply: bool,
        /// Token returned by the preview; refused if the selected set changes.
        #[arg(long, requires = "apply")]
        confirm: Option<String>,
    },
}

#[derive(Debug, Subcommand)]
pub enum BundleCommand {
    /// Save NAME=REF entries without duplicating their contents; print a short reference.
    Create {
        #[arg(required = true, num_args = 1..=100)]
        items: Vec<String>,
        #[arg(long)]
        json: bool,
    },
    /// Show a bounded item index without reading item payloads.
    Show {
        id: String,
        #[arg(long, default_value_t = 0)]
        offset: usize,
        #[arg(long, default_value_t = 20, value_parser = clap::value_parser!(u32).range(1..=100))]
        limit: u32,
    },
    /// Read just one item. Bytes are bounded JSON by default; --output exports exact bytes.
    Read {
        id: String,
        name: String,
        #[arg(long, default_value_t = 0)]
        offset: u64,
        #[arg(long, default_value_t = 4096, value_parser = clap::value_parser!(u32).range(1..=65536))]
        max_bytes: u32,
        #[arg(short, long, conflicts_with_all = ["offset", "max_bytes"])]
        output: Option<PathBuf>,
        #[arg(long, requires = "output")]
        force: bool,
    },
}

#[derive(Debug, Args)]
pub struct FilterArgs {
    /// Case-sensitive substring of the recorded source path.
    #[arg(long)]
    pub source: Option<String>,
    /// Case-sensitive substring of the saved note.
    #[arg(long)]
    pub note: Option<String>,
}

#[derive(Debug, Args)]
pub struct CopyArgs {
    /// Input file, or - for stdin. Omit the source when piping input. No selector means all bytes.
    pub source: Option<PathBuf>,
    #[command(flatten)]
    pub spec: Spec,
    /// Optional metadata note; never changes the saved bytes.
    #[arg(long)]
    pub note: Option<String>,
    /// Report locations and byte counts without saving or creating a store.
    #[arg(long)]
    pub dry_run: bool,
    /// Include an escaped content preview, bounded by --preview-bytes.
    #[arg(long, requires = "dry_run")]
    pub show_content: bool,
    #[arg(long, requires = "show_content", default_value_t = 256, value_parser = clap::value_parser!(u32).range(1..=4096))]
    pub preview_bytes: u32,
    /// Input read limit in bytes. Hard maximum: 134217728 (128 MiB).
    #[arg(long, default_value_t = DEFAULT_MAX_BYTES)]
    pub max_bytes: usize,
    /// Return JSON metadata and selected space on success instead of only the reference.
    #[arg(long)]
    pub json: bool,
}

#[derive(Debug, Args)]
pub struct RunArgs {
    /// Redact this environment variable's exact value from logs and stored argv before saving.
    #[arg(long)]
    pub redact_env: Vec<String>,
    /// Combined stdout/stderr byte cap (default 64 MiB; maximum 1 GiB).
    #[arg(long, env = "CPYREF_RUN_MAX_LOG_BYTES")]
    pub max_log_bytes: Option<u64>,
    /// Deadline including output-pipe draining (default 600 seconds; maximum 86400).
    #[arg(long, env = "CPYREF_RUN_TIMEOUT_SECONDS")]
    pub timeout: Option<u64>,
    /// Free-space reserve in bytes (default 512 MiB), plus log packing headroom.
    #[arg(long, env = "CPYREF_RUN_MIN_FREE_BYTES")]
    pub min_free_bytes: Option<u64>,
    #[arg(long, default_value = "auto", value_parser = ["auto", "cargo", "none"])]
    pub parser: String,
    /// Require the saved source manifest to match before starting the command.
    #[arg(long, conflicts_with = "input")]
    pub snapshot: Option<String>,
    /// Capture explicit source paths before the command; repeat for multiple paths.
    #[arg(long)]
    pub input: Vec<PathBuf>,
    /// Hash a declared output after the command; does not prove it was rebuilt.
    #[arg(long)]
    pub artifact: Vec<PathBuf>,
    /// Allow the child to read terminal input. Default stdin is closed.
    #[arg(long)]
    pub inherit_stdin: bool,
    #[arg(last = true, required = true, num_args = 1..)]
    pub command: Vec<String>,
}

#[derive(Debug, Args)]
#[command(args_conflicts_with_subcommands = true, subcommand_negates_reqs = true)]
pub struct SnapshotArgs {
    #[command(subcommand)]
    pub command: Option<SnapshotCommand>,
    pub paths: Vec<PathBuf>,
}

#[derive(Debug, Subcommand)]
pub enum SnapshotCommand {
    /// Compare the selected source tree with a manifest; exit 10 if changed.
    Check {
        id: String,
        #[arg(long, default_value_t = 20, value_parser = clap::value_parser!(u32).range(1..=1000))]
        limit: u32,
        #[arg(long, default_value_t = 0)]
        offset: usize,
    },
    /// Compare two saved manifests without reading current source; exit 10 if changed.
    Diff {
        before: String,
        after: String,
        #[arg(long, default_value_t = 20, value_parser = clap::value_parser!(u32).range(1..=1000))]
        limit: u32,
        #[arg(long, default_value_t = 0)]
        offset: usize,
    },
    /// Show a bounded page of saved source paths and hashes, never source bodies.
    Show {
        id: String,
        #[arg(long, default_value_t = 20, value_parser = clap::value_parser!(u32).range(1..=1000))]
        limit: u32,
        #[arg(long, default_value_t = 0)]
        offset: usize,
    },
}

#[cfg(test)]
mod tests {
    use super::*;
    use clap::CommandFactory;

    #[test]
    fn copy_selector_arguments_keep_defaults_and_constraints() {
        Cli::command().debug_assert();
        let parse = |flags: &[&str]| {
            let args = ["cpyref", "copy", "input.txt"].into_iter();
            Cli::try_parse_from(args.chain(flags.iter().copied()))
        };
        let Command::Copy(args) = parse(&[]).unwrap().command else {
            panic!("expected copy command");
        };
        assert_eq!(args.spec.context_lines, 3);
        assert_eq!(Spec::default().context_lines, 0);
        for flags in [
            vec!["--lines", "1:2"],
            vec!["--text", "word", "-i", "--within", "1:9"],
            vec!["--regex", "(word)", "--group", "1"],
            vec!["--from", "BEGIN", "--to", "END", "--exclude-end"],
        ] {
            assert!(parse(&flags).is_ok(), "{flags:?}");
        }
        for flags in [
            vec!["--from", "BEGIN"],
            vec!["--to", "END"],
            vec!["--group", "1"],
            vec!["--exclude-start"],
            vec![
                "--from",
                "B",
                "--to",
                "E",
                "--exclude-markers",
                "--exclude-end",
            ],
        ] {
            assert!(parse(&flags).is_err(), "{flags:?}");
        }
    }
}
