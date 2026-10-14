use clap::{Args, Parser, Subcommand};
use cpyref::{context::Options, select::Spec, DEFAULT_MAX_BYTES};
use std::path::PathBuf;

#[derive(Debug, Parser)]
#[command(
    version,
    about = "Copy exact bytes once. Paste them by a short reference."
)]
pub struct Cli {
    #[command(flatten)]
    pub context: Options,
    #[command(subcommand)]
    pub command: Command,
}

#[derive(Debug, Subcommand)]
pub enum Command {
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
    /// Generate a complete same-host retrieval command or structured handoff.
    Handoff {
        id: String,
        #[arg(long, default_value = "json", value_parser = ["json", "sh", "powershell"])]
        format: String,
    },
    /// Save a unique selection; print only its short reference by default.
    Copy(Box<CopyArgs>),
    /// Write the saved bytes, without adding a newline.
    Paste {
        id: String,
        /// Write to a file instead of stdout. Existing files are protected by default.
        #[arg(short, long)]
        output: Option<PathBuf>,
        /// Replace an existing regular output file.
        #[arg(long, requires = "output")]
        force: bool,
    },
    /// Print JSON metadata without expanding the copied content.
    Info { id: String },
    /// Delete one saved snapshot by its complete reference.
    #[command(alias = "rm")]
    Remove { id: String },
    /// Show the selected project, namespace, store, and config files; never creates a store.
    Context,
    /// List metadata in this space, newest first; never expand payloads.
    List {
        #[command(flatten)]
        filter: FilterArgs,
        #[arg(long, default_value_t = 20, value_parser = clap::value_parser!(u32).range(1..=1000))]
        limit: u32,
        #[arg(long, default_value_t = 0)]
        offset: usize,
        #[arg(long)]
        json: bool,
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
    /// Input file, or - to read bytes from stdin. No selector means the whole input.
    pub source: PathBuf,
    /// One-based inclusive lines: N or START:END. Original line endings are retained.
    #[arg(long)]
    pub lines: Option<String>,
    /// Match exact text, including overlapping matches.
    #[arg(long)]
    pub text: Option<String>,
    /// Match a Rust regex. Matches are leftmost and non-overlapping.
    #[arg(long)]
    pub regex: Option<String>,
    /// Start marker; pair with its nearest following end marker.
    #[arg(long, requires = "to")]
    pub from: Option<String>,
    #[arg(long, requires = "from")]
    pub to: Option<String>,
    /// Interpret both markers as regexes instead of literal text.
    #[arg(long, requires = "from")]
    pub markers_regex: bool,
    /// Copy only content between the markers, without the markers themselves.
    #[arg(long, requires = "from")]
    pub exclude_markers: bool,
    /// Exclude only the start marker (both markers are included by default).
    #[arg(long, requires = "from", conflicts_with = "exclude_markers")]
    pub exclude_start: bool,
    /// Exclude only the end marker, keeping the current function declaration.
    #[arg(long, requires = "from", conflicts_with = "exclude_markers")]
    pub exclude_end: bool,
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
    /// Copy a numbered or named regex capture group, not the complete match.
    #[arg(long, requires = "regex")]
    pub group: Option<String>,
    /// Restrict all searching to these absolute, one-based inclusive lines.
    #[arg(long)]
    pub within: Option<String>,
    /// Require exact, case-sensitive text in the preceding context window.
    #[arg(long)]
    pub before: Option<String>,
    /// Require exact, case-sensitive text in the following context window.
    #[arg(long)]
    pub after: Option<String>,
    /// Extra lines on each side for context filters, in addition to the same line.
    #[arg(long, default_value_t = 3)]
    pub context_lines: usize,
    /// Explicitly select the Nth match after scope and context filtering (starts at 1).
    #[arg(long)]
    pub occurrence: Option<usize>,
    /// Case-insensitive selector matching. Original bytes are never case-converted.
    #[arg(short = 'i', long)]
    pub ignore_case: bool,
    /// Make regex ^ and $ match line boundaries.
    #[arg(long)]
    pub multiline: bool,
    /// Make regex . also match newlines.
    #[arg(long)]
    pub dotall: bool,
    /// Input read limit in bytes. Hard maximum: 134217728 (128 MiB).
    #[arg(long, default_value_t = DEFAULT_MAX_BYTES)]
    pub max_bytes: usize,
    /// Return JSON metadata and selected space on success instead of only the reference.
    #[arg(long)]
    pub json: bool,
}

impl CopyArgs {
    pub fn spec(&self) -> Spec {
        Spec {
            lines: self.lines.clone(),
            text: self.text.clone(),
            regex: self.regex.clone(),
            from: self.from.clone(),
            to: self.to.clone(),
            markers_regex: self.markers_regex,
            exclude_markers: self.exclude_markers,
            exclude_start: self.exclude_start,
            exclude_end: self.exclude_end,
            group: self.group.clone(),
            within: self.within.clone(),
            before: self.before.clone(),
            after: self.after.clone(),
            context_lines: self.context_lines,
            occurrence: self.occurrence,
            ignore_case: self.ignore_case,
            multiline: self.multiline,
            dotall: self.dotall,
        }
    }
}

#[derive(Debug, Args)]
pub struct RunArgs {
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
