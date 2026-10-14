use clap::{Args, Parser, Subcommand};
use cpyref::{context::Options, select::Spec, DEFAULT_MAX_BYTES};
use std::path::PathBuf;

#[derive(Debug, Parser)]
#[command(version, about = "Copy exact bytes once. Paste them by a short reference.")]
pub struct Cli {
    #[command(flatten)]
    pub context: Options,
    #[command(subcommand)]
    pub command: Command,
}

#[derive(Debug, Subcommand)]
pub enum Command {
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
