//! Steering hints that follow tool use across a whole session. A hint appears only when a tool is
//! used where another one answers the same question for fewer tokens, so an agent that is on
//! course never sees one. Each hint fires once per streak and is not repeated while it is ignored.

use std::collections::BTreeMap;

use crate::filesystem::tools::GrepParams;
use crate::langtool::engine::reader::UnfocusedRead;
use crate::language::registry;
use crate::workflow::tools::PatchMiss;

/// Symbol searches by `filesystem_grep` in a row after which `langtool_find_references` is
/// suggested.
const SYMBOL_GREPS_BEFORE_HINT: usize = 2;
/// `filesystem_ls` calls on a directory followed by `langtool_find_symbols` on it without a
/// summary in between, at which the hint suggests `langtool_summarize_codebase`.
const LS_BEFORE_SUMMARIZE_HINT: usize = 1;
/// Lookups of one name in a row by `langtool_find_symbols`, `langtool_find_references` and
/// `langtool_view_callgraph` at which the hint suggests `langtool_summarize_symbol`.
const LOOKUPS_BEFORE_SUMMARIZE_SYMBOL_HINT: usize = 3;
/// Searches by words, not by a code identifier, in a row at which the hint suggests
/// `langtool_summarize_feature`.
const WORD_GREPS_BEFORE_FEATURE_HINT: usize = 3;
/// Hints about raw reads that may go unheeded before they stop, until the agent reads an outline
/// or a symbol.
const IGNORED_READ_HINTS_LIMIT: usize = 2;
/// Names shorter than this are more likely words than code symbols.
const MIN_SYMBOL_LEN: usize = 3;
/// The failed patch on one file in a turn at which the hint says where to read it again.
const PATCH_MISS_OF_HINT: usize = 2;

#[derive(Default)]
pub(crate) struct ToolUseHints {
    symbol_greps: usize,
    grep_hinted: bool,
    ignored_read_hints: usize,
    /// Count of `filesystem_ls` calls on directories since the last summary or symbol call.
    ls_streak: usize,
    ls_hinted: bool,
    /// The bare name, lowercased, that the current run of symbol lookups is about.
    lookup_name: String,
    /// Symbol lookups of `lookup_name` in a row.
    lookups: usize,
    lookup_hinted: bool,
    /// Greps for words rather than identifiers since the last summary.
    word_greps: usize,
    word_hinted: bool,
    /// The turn that `patch_misses` counts in.
    miss_turn: String,
    /// Failed patches per file since a write last changed the file, in turn `miss_turn`.
    patch_misses: BTreeMap<String, usize>,
}

impl ToolUseHints {
    pub(crate) fn reset(&mut self) {
        *self = Self::default();
    }

    /// A `filesystem_ls` ran on a directory. After one such call without an intervening summary,
    /// suggest `langtool_summarize_codebase`, once per streak.
    pub(crate) fn after_ls(&mut self, is_dir: bool) -> Option<String> {
        if !is_dir {
            self.ls_streak = 0;
            self.ls_hinted = false;
            return None;
        }
        self.ls_streak += 1;
        if self.ls_streak <= LS_BEFORE_SUMMARIZE_HINT || self.ls_hinted {
            return None;
        }
        self.ls_hinted = true;
        Some(
            "langtool_summarize_codebase gives layers, doors and the start path in one call"
                .to_string(),
        )
    }

    /// A summary tool ran: reset the ls streak.
    pub(crate) fn end_ls_streak(&mut self) {
        self.ls_streak = 0;
        self.ls_hinted = false;
    }

    /// A symbol lookup ran for `names`. The third lookup of one name in a row suggests
    /// `langtool_summarize_symbol`, once per run. Several names, or a different name, start over.
    pub(crate) fn after_symbol_lookup(&mut self, names: &[String]) -> Option<String> {
        let [name] = names else {
            self.end_lookup_streak();
            return None;
        };
        let bare = name
            .rsplit(|c: char| matches!(c, ':' | '.' | '/'))
            .next()
            .unwrap_or(name)
            .to_lowercase();
        if bare.is_empty() {
            self.end_lookup_streak();
            return None;
        }
        if bare != self.lookup_name {
            self.lookup_name = bare;
            self.lookups = 0;
            self.lookup_hinted = false;
        }
        self.lookups += 1;
        if self.lookups < LOOKUPS_BEFORE_SUMMARIZE_SYMBOL_HINT || self.lookup_hinted {
            return None;
        }
        self.lookup_hinted = true;
        Some(
            "langtool_summarize_symbol gives the definition, what it uses and what uses it in one call"
                .to_string(),
        )
    }

    /// A summary tool ran, or a lookup of several names: the run of lookups is over.
    pub(crate) fn end_lookup_streak(&mut self) {
        self.lookup_name.clear();
        self.lookups = 0;
        self.lookup_hinted = false;
        self.word_greps = 0;
        self.word_hinted = false;
    }

    /// A grep ran; `symbol_search` says whether it looked up a code identifier in code files.
    /// The second such search in a row suggests the symbol tools, once for the streak.
    pub(crate) fn after_grep(&mut self, symbol_search: bool) -> Option<String> {
        if !symbol_search {
            self.end_grep_streak();
            self.word_greps += 1;
            if self.word_greps < WORD_GREPS_BEFORE_FEATURE_HINT || self.word_hinted {
                return None;
            }
            self.word_hinted = true;
            return Some(
                "langtool_summarize_feature finds the code behind a behaviour from words in one call"
                    .to_string(),
            );
        }
        self.symbol_greps += 1;
        if self.symbol_greps < SYMBOL_GREPS_BEFORE_HINT || self.grep_hinted {
            return None;
        }
        self.grep_hinted = true;
        Some(
            "for code symbols, langtool_find_references skips string and comment noise and shows the caller scopes."
                .to_string(),
        )
    }

    /// A search that is not for a symbol, or a symbol-aware tool, ends the grep streak.
    pub(crate) fn end_grep_streak(&mut self) {
        self.symbol_greps = 0;
        self.grep_hinted = false;
    }

    /// A file read ran; `focused` says whether some entry asked for an outline or a symbol, and
    /// `unfocused` is the raw read of a long code file, if any. A focused read starts the hints
    /// over; after two unheeded hints they stop.
    pub(crate) fn after_read(
        &mut self,
        focused: bool,
        unfocused: Option<UnfocusedRead>,
    ) -> Option<String> {
        if focused {
            self.ignored_read_hints = 0;
        }
        let raw = unfocused?;
        if self.ignored_read_hints >= IGNORED_READ_HINTS_LIMIT {
            return None;
        }
        self.ignored_read_hints += 1;
        Some(format!(
            "{} has {} lines; fold_level=0 gives an outline, symbol='Name' reads one definition, \
             langtool_summarize_symbol shows what it uses and what uses it.",
            raw.path, raw.lines
        ))
    }

    /// A `Patch` did not place in `turn`. The second miss on one file in a turn says, once, where to
    /// read it again: the lines of the definition around the failing hunk, or of the hunk itself.
    pub(crate) fn after_patch_miss(&mut self, turn: &str, miss: &PatchMiss) -> Option<String> {
        if self.miss_turn != turn {
            self.miss_turn = turn.to_string();
            self.patch_misses.clear();
        }
        let misses = self.patch_misses.entry(miss.path.clone()).or_insert(0);
        *misses += 1;
        if *misses != PATCH_MISS_OF_HINT {
            return None;
        }
        Some(match (miss.lines, miss.symbol.as_deref()) {
            (Some((first, last)), Some(symbol)) => {
                format!("re-read lines {first}..{last}, or ReplaceSymbol {symbol}")
            }
            (Some((first, last)), None) => {
                format!("re-read lines {first}..{last} before the next Patch")
            }
            (None, _) => format!("re-read {} before the next Patch", miss.path),
        })
    }

    /// A write changed `paths`: the patch misses on them are over.
    pub(crate) fn patches_applied(&mut self, paths: &[String]) {
        for path in paths {
            self.patch_misses.remove(path);
        }
    }
}

/// `out` with `hint` as its last line.
pub(crate) fn hinted(mut out: String, hint: Option<String>) -> String {
    if let Some(hint) = hint {
        if !out.ends_with('\n') {
            out.push('\n');
        }
        out.push_str(&format!("hint: {hint}\n"));
    }
    out
}

/// Whether a grep looks up a code identifier (`put`, `Store::put`, `MAX_LINES`) in code files.
pub(crate) fn is_symbol_search(p: &GrepParams) -> bool {
    looks_like_symbol(p.query.trim()) && searches_code(p)
}

fn looks_like_symbol(query: &str) -> bool {
    let plain = |segment: &str| {
        let mut chars = segment.chars();
        chars.next().is_some_and(|c| c.is_alphabetic() || c == '_')
            && chars.all(|c| c.is_alphanumeric() || c == '_')
    };
    let name = query.rsplit("::").next().unwrap_or(query);
    query.split("::").all(plain)
        && name.chars().count() >= MIN_SYMBOL_LEN
        && (name.contains('_') || name.chars().any(char::is_lowercase))
}

/// Whether the files a grep reaches can be code: its globs or its path name a language with a
/// syntax, or name no extension at all.
fn searches_code(p: &GrepParams) -> bool {
    let is_code = |path: &str| {
        extension(path).is_none_or(|ext| registry::for_path(&format!("f.{ext}")).is_some())
    };
    match p.glob.as_deref() {
        Some(globs) if !globs.is_empty() => globs.iter().any(|glob| is_code(glob)),
        _ => p.implicit_path.as_deref().is_none_or(is_code),
    }
}

/// The extension of the last component of a path or glob, when it has one.
fn extension(path: &str) -> Option<&str> {
    let name = path.rsplit(['/', '\\']).next()?;
    name.rsplit_once('.')
        .map(|(_, ext)| ext)
        .filter(|ext| !ext.is_empty())
}
