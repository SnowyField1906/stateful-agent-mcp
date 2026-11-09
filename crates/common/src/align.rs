//! Matching quoted lines against file lines when only whitespace differs.
//!
//! A quote of file text (an agent's `old_text`) is often right line by line and wrong in
//! whitespace: trailing blanks, a block indented one level too deep or too shallow, tabs where the
//! file has spaces, runs of blanks inside a line. [`find_fits`] finds where such a quote sits and
//! [`reindent`] writes the replacement in the file's own indentation. Everything here works on
//! lines without their terminators and knows nothing about syntax: whether a fit may be trusted
//! inside a string literal is for the caller to decide.
//!
//! [`diagnose`] is for a quote that fits nowhere: it finds the window of the file the quote was
//! aimed at and aligns the two line by line, so that the lines that differ can be shown. It never
//! decides that a quote may be placed.

use std::collections::{BTreeMap, HashMap};

/// `line` without its terminator (`\n` or `\r\n`).
pub fn body(line: &str) -> &str {
    let line = line.strip_suffix('\n').unwrap_or(line);
    line.strip_suffix('\r').unwrap_or(line)
}

/// The blanks `line` starts with.
pub fn indent_of(line: &str) -> &str {
    &line[..line.len() - line.trim_start().len()]
}

/// An indentation change that holds for every non-blank line of a block.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Shift {
    /// The file indents each line with this much more than the quote.
    Add(String),
    /// The quote indents each line with this much more than the file.
    Strip(String),
    /// The quote indents with spaces and the file with tabs; `per` spaces are one tab.
    SpacesToTabs { per: usize },
    /// The quote indents with tabs and the file with spaces; one tab is `per` spaces.
    TabsToSpaces { per: usize },
}

impl Shift {
    /// The file's indentation for a quote line indented with `indent`; `None` when the change
    /// does not apply to that line.
    pub fn to_file(&self, indent: &str) -> Option<String> {
        match self {
            Shift::Add(prefix) => Some(format!("{prefix}{indent}")),
            Shift::Strip(prefix) => indent.strip_prefix(prefix.as_str()).map(str::to_string),
            Shift::SpacesToTabs { per } => {
                if indent.bytes().all(|b| b == b' ') && indent.len() % per == 0 {
                    Some("\t".repeat(indent.len() / per))
                } else {
                    None
                }
            }
            Shift::TabsToSpaces { per } => {
                if indent.bytes().all(|b| b == b'\t') {
                    Some(" ".repeat(indent.len() * per))
                } else {
                    None
                }
            }
        }
    }

    /// Short words for a report: `indent +4 spaces`.
    pub fn describe(&self) -> String {
        match self {
            Shift::Add(prefix) => format!("indent +{}", blanks(prefix)),
            Shift::Strip(prefix) => format!("indent -{}", blanks(prefix)),
            Shift::SpacesToTabs { .. } => "indent spaces → tabs".to_string(),
            Shift::TabsToSpaces { .. } => "indent tabs → spaces".to_string(),
        }
    }
}

fn blanks(ws: &str) -> String {
    let n = ws.chars().count();
    let plural = if n == 1 { "" } else { "s" };
    if ws.chars().all(|c| c == ' ') {
        format!("{n} space{plural}")
    } else if ws.chars().all(|c| c == '\t') {
        format!("{n} tab{plural}")
    } else {
        format!("{n} blank{plural}")
    }
}

/// How two lines are compared once their indentation and trailing blanks are set aside.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Compare {
    /// The lines are equal.
    Trimmed,
    /// The lines are equal when each run of blanks inside them counts as one blank. A blank that
    /// one line has and the other lacks still differs.
    Squeezed,
}

impl Compare {
    fn same(self, quoted: &str, filed: &str) -> bool {
        match self {
            Compare::Trimmed => quoted.trim() == filed.trim(),
            Compare::Squeezed => quoted.split_whitespace().eq(filed.split_whitespace()),
        }
    }
}

/// How a quote's lines relate to the file lines they were found at.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Fit {
    /// The indentation change applied to every non-blank line; `None` when indentation is the
    /// same and only trailing blanks differ.
    pub shift: Option<Shift>,
}

/// The change of indentation that turns the quote's indentation `q` into the file's `f`.
/// `Some(None)` is no change; `None` is a difference that no single rule describes.
fn candidate(q: &str, f: &str) -> Option<Option<Shift>> {
    if q == f {
        return Some(None);
    }
    if f.len() > q.len() && f.ends_with(q) {
        return Some(Some(Shift::Add(f[..f.len() - q.len()].to_string())));
    }
    if q.len() > f.len() && q.ends_with(f) {
        return Some(Some(Shift::Strip(q[..q.len() - f.len()].to_string())));
    }
    let spaces = |s: &str| !s.is_empty() && s.bytes().all(|b| b == b' ');
    let tabs = |s: &str| !s.is_empty() && s.bytes().all(|b| b == b'\t');
    if spaces(q) && tabs(f) && q.len() % f.len() == 0 {
        let per = q.len() / f.len();
        if (2..=8).contains(&per) {
            return Some(Some(Shift::SpacesToTabs { per }));
        }
    }
    if tabs(q) && spaces(f) && f.len() % q.len() == 0 {
        let per = f.len() / q.len();
        if (2..=8).contains(&per) {
            return Some(Some(Shift::TabsToSpaces { per }));
        }
    }
    None
}

/// Whether the quote's lines are the file's lines up to trailing blanks, one indentation
/// change that holds for every non-blank line, and `compare`. Blank lines only need to be blank
/// in both.
pub fn fit_lines(quote: &[&str], file: &[&str], compare: Compare) -> Option<Fit> {
    if quote.len() != file.len() {
        return None;
    }
    let mut decided: Option<Option<Shift>> = None;
    for (q, f) in quote.iter().zip(file) {
        if !compare.same(q, f) {
            return None;
        }
        if q.trim().is_empty() {
            continue;
        }
        let (quote_indent, file_indent) = (indent_of(q), indent_of(f));
        if decided.is_none() {
            decided = Some(candidate(quote_indent, file_indent)?);
        }
        let expected = match decided.as_ref().and_then(Option::as_ref) {
            None => Some(quote_indent.to_string()),
            Some(shift) => shift.to_file(quote_indent),
        };
        if expected.as_deref() != Some(file_indent) {
            return None;
        }
    }
    Some(Fit {
        shift: decided.flatten(),
    })
}

/// Every start line (0-based) where the quote fits the file lines, with how it fits. A quote
/// without a non-blank line fits nowhere.
pub fn find_fits(quote: &[&str], file: &[&str], compare: Compare) -> Vec<(usize, Fit)> {
    let Some(key) = quote.iter().position(|line| !line.trim().is_empty()) else {
        return Vec::new();
    };
    if quote.len() > file.len() {
        return Vec::new();
    }
    let first = quote[key];
    let mut fits = Vec::new();
    for start in 0..=file.len() - quote.len() {
        if !compare.same(first, file[start + key]) {
            continue;
        }
        if let Some(fit) = fit_lines(quote, &file[start..start + quote.len()], compare) {
            fits.push((start, fit));
        }
    }
    fits
}

/// `text` with every non-blank line's indentation changed by `shift`, so that text written for
/// the quote reads right in the file. `None` when a line cannot take the change (it would have
/// to lose indentation it does not have).
pub fn reindent(text: &str, shift: Option<&Shift>) -> Option<String> {
    let Some(shift) = shift else {
        return Some(text.to_string());
    };
    let mut out = String::with_capacity(text.len());
    for line in text.split_inclusive('\n') {
        let content = body(line);
        if content.trim().is_empty() {
            out.push_str(line);
            continue;
        }
        let indent = indent_of(content);
        out.push_str(&shift.to_file(indent)?);
        out.push_str(&content[indent.len()..]);
        out.push_str(&line[content.len()..]);
    }
    Some(out)
}

/// A line as the diagnosis compares it: trimmed, each run of blanks squeezed to one blank.
pub fn normal(line: &str) -> String {
    line.split_whitespace().collect::<Vec<_>>().join(" ")
}

fn is_word(token: &str) -> bool {
    token
        .chars()
        .next()
        .is_some_and(|c| c.is_alphanumeric() || c == '_')
}

/// The words and single marks of a normalized line: `f(a, b)` is `f ( a , b )`.
fn tokens(line: &str) -> Vec<&str> {
    let mut out = Vec::new();
    let mut start: Option<usize> = None;
    for (i, c) in line.char_indices() {
        let word = c.is_alphanumeric() || c == '_';
        if word {
            if start.is_none() {
                start = Some(i);
            }
            continue;
        }
        if let Some(s) = start.take() {
            out.push(&line[s..i]);
        }
        if !c.is_whitespace() {
            out.push(&line[i..i + c.len_utf8()]);
        }
    }
    if let Some(s) = start {
        out.push(&line[s..]);
    }
    out
}

/// Longest line, in characters, that the edit distance looks at.
const MAX_CHARS: usize = 200;

/// `1 - edit distance / length` of two lines. Lines whose lengths differ by more than half the
/// longer one are not alike at all.
fn edit_similarity(a: &str, b: &str) -> f32 {
    let a: Vec<char> = a.chars().take(MAX_CHARS).collect();
    let b: Vec<char> = b.chars().take(MAX_CHARS).collect();
    let longest = a.len().max(b.len());
    if longest == 0 {
        return 1.0;
    }
    if a.len().abs_diff(b.len()) * 2 > longest {
        return 0.0;
    }
    let mut row: Vec<usize> = (0..=b.len()).collect();
    for i in 1..=a.len() {
        let mut diagonal = row[0];
        row[0] = i;
        for j in 1..=b.len() {
            let above = row[j];
            let cost = usize::from(a[i - 1] != b[j - 1]);
            row[j] = (above + 1).min(row[j - 1] + 1).min(diagonal + cost);
            diagonal = above;
        }
    }
    1.0 - row[b.len()] as f32 / longest as f32
}

/// How alike two normalized lines are, from 0.0 to 1.0: the share of words and marks they have in
/// common, blended with their edit distance. Lines that have words and share none are 0.
pub fn similarity(a: &str, b: &str) -> f32 {
    if a == b {
        return 1.0;
    }
    if a.is_empty() || b.is_empty() {
        return 0.0;
    }
    let mut left = tokens(a);
    left.sort_unstable();
    left.dedup();
    let mut right = tokens(b);
    right.sort_unstable();
    right.dedup();
    let (mut common, mut common_words) = (0usize, 0usize);
    for token in &left {
        if right.binary_search(token).is_ok() {
            common += 1;
            if is_word(token) {
                common_words += 1;
            }
        }
    }
    let has_words = left.iter().chain(right.iter()).any(|token| is_word(token));
    if has_words && common_words == 0 {
        return 0.0;
    }
    let union = left.len() + right.len() - common;
    let jaccard = if union == 0 {
        0.0
    } else {
        common as f32 / union as f32
    };
    (jaccard + edit_similarity(a, b)) / 2.0
}

/// How far the lines of a window may stray from the diagonal a quote's anchors voted for.
const BAND: usize = 8;
/// Longest quote, in lines, that is aligned.
const MAX_QUOTE: usize = 300;
/// Share of a quote's non-blank lines that a window has to align for the report to name it.
const FLOOR: u8 = 50;
/// Least similarity at which two lines are taken for the same line, edited.
const PAIRED_FROM: f32 = 0.45;
/// Most lines of extra context that [`extensions`] tries.
const MAX_EXTENSION: usize = 12;

/// How one line of a quote stands to the window of the file it was aligned with. Indexes count
/// the lines of the slices given to [`diagnose`].
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Pair {
    /// The lines are equal up to blanks.
    Same { quote: usize, file: usize },
    /// The lines are alike but differ.
    Edit { quote: usize, file: usize },
    /// A quoted line with no counterpart in the window.
    Extra { quote: usize },
    /// A line of the window, between two aligned lines, that the quote does not hold.
    Gap { file: usize },
}

/// What is wrong with a quote, read off its alignment.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Tag {
    /// A line differs.
    Edit,
    /// Lines that are apart in the file are quoted as adjacent.
    Gap,
    /// A quoted line is not in the window.
    Extra,
    /// Quoted lines are in the window, in another order.
    Order,
}

impl Tag {
    pub fn word(self) -> &'static str {
        match self {
            Tag::Edit => "edit",
            Tag::Gap => "gap",
            Tag::Extra => "extra",
            Tag::Order => "order",
        }
    }
}

/// The window of the file that a quote was aimed at, and the lines of the two side by side.
#[derive(Clone, Debug)]
pub struct Diagnosis {
    /// First and last line of the window, 0-based and inclusive: the first and last file lines
    /// that the quote aligned with.
    pub first: usize,
    pub last: usize,
    /// The share of the quote's non-blank lines that were aligned, in percent.
    pub percent: u8,
    pub pairs: Vec<Pair>,
    /// Other windows that fit as well as this one: `(first, last)` each.
    pub rivals: Vec<(usize, usize)>,
}

impl Diagnosis {
    /// Quoted lines that equal a window line the quote skipped: `(quote line, file line)`. The lines
    /// are there, in another order.
    pub fn swapped(&self, quote: &[&str], file: &[&str]) -> Vec<(usize, usize)> {
        let mut used: Vec<usize> = Vec::new();
        let mut out = Vec::new();
        for pair in &self.pairs {
            let Pair::Extra { quote: q } = *pair else {
                continue;
            };
            let want = normal(quote[q]);
            if want.is_empty() {
                continue;
            }
            let hit = self.pairs.iter().find_map(|p| match *p {
                Pair::Gap { file: f } if !used.contains(&f) && normal(file[f]) == want => Some(f),
                _ => None,
            });
            if let Some(f) = hit {
                used.push(f);
                out.push((q, f));
            }
        }
        out
    }

    /// What differs, from the alignment itself: lines edited, lines apart, lines not in the window
    /// and lines in another order.
    pub fn tags(&self, quote: &[&str], file: &[&str]) -> Vec<Tag> {
        let swapped = self.swapped(quote, file);
        let (mut edit, mut gap, mut extra) = (false, false, false);
        for pair in &self.pairs {
            match *pair {
                Pair::Edit { .. } => edit = true,
                Pair::Gap { file: f } => gap |= !swapped.iter().any(|&(_, g)| g == f),
                Pair::Extra { quote: q } => extra |= !swapped.iter().any(|&(e, _)| e == q),
                Pair::Same { .. } => {}
            }
        }
        let mut tags = Vec::new();
        if edit {
            tags.push(Tag::Edit);
        }
        if gap {
            tags.push(Tag::Gap);
        }
        if extra {
            tags.push(Tag::Extra);
        }
        if !swapped.is_empty() {
            tags.push(Tag::Order);
        }
        tags
    }
}

/// Find where in `file` the lines of `quote` were aimed and align the two. Lines are compared
/// trimmed and with blanks squeezed. `None` when no window aligns at least half of the quote's
/// non-blank lines (a quote with none, or one too long, has no window either).
///
/// Candidate windows come from anchors: quote lines that are unique in the quote and occur at most
/// twice in the file vote for the diagonal they sit on, and the three best diagonals are aligned.
/// A quote with no anchor votes by its longest line's nearest lines instead.
pub fn diagnose(quote: &[&str], file: &[&str]) -> Option<Diagnosis> {
    if quote.is_empty() || quote.len() > MAX_QUOTE || file.is_empty() {
        return None;
    }
    let nq: Vec<String> = quote.iter().map(|line| normal(line)).collect();
    let nf: Vec<String> = file.iter().map(|line| normal(line)).collect();
    if nq.iter().all(String::is_empty) {
        return None;
    }
    let mut ranked: Vec<(u8, f32, Diagnosis)> = Vec::new();
    for diagonal in candidate_diagonals(&nq, &nf) {
        if let Some((score, found)) = align_window(&nq, &nf, diagonal) {
            ranked.push((found.percent, score, found));
        }
    }
    ranked.sort_by(|a, b| {
        b.0.cmp(&a.0)
            .then(b.1.total_cmp(&a.1))
            .then(a.2.first.cmp(&b.2.first))
    });
    if ranked.is_empty() {
        return None;
    }
    let (percent, score, mut best) = ranked.remove(0);
    if percent < FLOOR {
        return None;
    }
    let mut rivals: Vec<(usize, usize)> = ranked
        .iter()
        .filter(|(p, s, d)| *p == percent && (*s - score).abs() < 1e-3 && d.first != best.first)
        .map(|(_, _, d)| (d.first, d.last))
        .collect();
    rivals.sort_unstable();
    rivals.dedup();
    best.rivals = rivals;
    Some(best)
}

/// The diagonals (`file line - quote line`) worth aligning along: up to three, best first.
fn candidate_diagonals(nq: &[String], nf: &[String]) -> Vec<isize> {
    let mut in_file: HashMap<&str, Vec<usize>> = HashMap::new();
    for (j, line) in nf.iter().enumerate() {
        if !line.is_empty() {
            in_file.entry(line.as_str()).or_default().push(j);
        }
    }
    let mut in_quote: HashMap<&str, usize> = HashMap::new();
    for line in nq {
        *in_quote.entry(line.as_str()).or_default() += 1;
    }
    let mut votes: BTreeMap<isize, usize> = BTreeMap::new();
    for (i, line) in nq.iter().enumerate() {
        if line.is_empty() || in_quote.get(line.as_str()).copied().unwrap_or(0) != 1 {
            continue;
        }
        let Some(at) = in_file.get(line.as_str()) else {
            continue;
        };
        if at.len() > 2 {
            continue;
        }
        for &j in at {
            *votes.entry(j as isize - i as isize).or_default() += 1;
        }
    }
    if votes.is_empty() {
        let longest = nq
            .iter()
            .enumerate()
            .max_by_key(|(_, line)| line.chars().count());
        if let Some((i, line)) = longest {
            let mut near: Vec<(f32, usize)> = nf
                .iter()
                .enumerate()
                .map(|(j, other)| (similarity(line, other), j))
                .filter(|(sim, _)| *sim >= 0.6)
                .collect();
            near.sort_by(|a, b| b.0.total_cmp(&a.0).then(a.1.cmp(&b.1)));
            for (_, j) in near.into_iter().take(3) {
                *votes.entry(j as isize - i as isize).or_default() += 1;
            }
        }
    }
    let band = BAND as isize;
    let mut scored: Vec<(usize, isize)> = votes
        .keys()
        .map(|&d| {
            let near: usize = votes.range(d - band..=d + band).map(|(_, v)| *v).sum();
            (near, d)
        })
        .collect();
    scored.sort_by(|a, b| b.0.cmp(&a.0).then(a.1.cmp(&b.1)));
    let mut picked: Vec<isize> = Vec::new();
    for (_, d) in scored {
        if picked.iter().all(|&p| (p - d).abs() > band) {
            picked.push(d);
        }
        if picked.len() == 3 {
            break;
        }
    }
    picked
}

/// The score of taking two normalized lines for one line; `None` when they are too different.
fn pair_score(q: &str, f: &str) -> Option<f32> {
    if q.is_empty() && f.is_empty() {
        return Some(0.2);
    }
    if q.is_empty() || f.is_empty() {
        return None;
    }
    let sim = similarity(q, f);
    (sim >= PAIRED_FROM).then_some(2.0 * sim - 1.0)
}

/// The cost of leaving a line unaligned: little for a blank line, which proves nothing.
fn skip_cost(line: &str) -> f32 {
    if line.is_empty() { 0.15 } else { 0.7 }
}

/// Align the whole quote with the lines of the file around `diagonal`. The file lines before and
/// after the aligned ones are free, the ones between them cost. `None` when nothing aligns.
fn align_window(nq: &[String], nf: &[String], diagonal: isize) -> Option<(f32, Diagnosis)> {
    let band = BAND as isize;
    let from = (diagonal - band).max(0) as usize;
    let to = ((diagonal + nq.len() as isize + band).max(0) as usize).min(nf.len());
    if from >= to {
        return None;
    }
    let win = &nf[from..to];
    let (n, m) = (nq.len(), win.len());
    let width = m + 1;
    let mut score = vec![f32::NEG_INFINITY; (n + 1) * width];
    // 1: the lines are a pair, 2: the quote line is extra, 3: the window line is a gap.
    let mut step = vec![0u8; (n + 1) * width];
    for cell in score.iter_mut().take(width) {
        *cell = 0.0;
    }
    for i in 1..=n {
        for j in 0..=m {
            let here = i * width + j;
            let mut best = f32::NEG_INFINITY;
            let mut how = 0u8;
            if j > 0
                && let Some(pair) = pair_score(&nq[i - 1], &win[j - 1])
            {
                best = score[(i - 1) * width + j - 1] + pair;
                how = 1;
            }
            let extra = score[(i - 1) * width + j] - skip_cost(&nq[i - 1]);
            if extra > best {
                best = extra;
                how = 2;
            }
            if j > 0 {
                let gap = score[here - 1] - skip_cost(&win[j - 1]);
                if gap > best {
                    best = gap;
                    how = 3;
                }
            }
            score[here] = best;
            step[here] = how;
        }
    }
    let mut end = 0;
    for j in 1..=m {
        if score[n * width + j] > score[n * width + end] {
            end = j;
        }
    }
    let total = score[n * width + end];
    let mut pairs: Vec<Pair> = Vec::new();
    let (mut i, mut j) = (n, end);
    while i > 0 {
        match step[i * width + j] {
            1 => {
                let (quote, file) = (i - 1, from + j - 1);
                pairs.push(if nq[i - 1] == win[j - 1] {
                    Pair::Same { quote, file }
                } else {
                    Pair::Edit { quote, file }
                });
                i -= 1;
                j -= 1;
            }
            3 => {
                pairs.push(Pair::Gap { file: from + j - 1 });
                j -= 1;
            }
            _ => {
                pairs.push(Pair::Extra { quote: i - 1 });
                i -= 1;
            }
        }
    }
    pairs.reverse();
    let aligned = |p: &Pair| matches!(p, Pair::Same { .. } | Pair::Edit { .. });
    let first_at = pairs.iter().position(aligned)?;
    let last_at = pairs.iter().rposition(aligned)?;
    let kept: Vec<Pair> = pairs
        .iter()
        .enumerate()
        .filter(|(k, p)| !matches!(p, Pair::Gap { .. }) || (first_at < *k && *k < last_at))
        .map(|(_, p)| *p)
        .collect();
    let files = |p: &Pair| match *p {
        Pair::Same { file, .. } | Pair::Edit { file, .. } => Some(file),
        _ => None,
    };
    let first = kept.iter().find_map(files)?;
    let last = kept.iter().rev().find_map(files)?;
    let solid = nq.iter().filter(|line| !line.is_empty()).count();
    if solid == 0 {
        return None;
    }
    let hit = kept
        .iter()
        .filter(|p| match **p {
            Pair::Same { quote, .. } | Pair::Edit { quote, .. } => !nq[quote].is_empty(),
            _ => false,
        })
        .count();
    let percent = (hit * 100 / solid) as u8;
    Some((
        total,
        Diagnosis {
            first,
            last,
            percent,
            pairs: kept,
            rivals: Vec::new(),
        },
    ))
}

/// Lines of context that tell one of several equal blocks from the others.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Extension {
    pub above: usize,
    pub below: usize,
}

/// For blocks of `len` lines that start at the 0-based lines `starts` of `file` and read alike:
/// the fewest lines of context, above and below, that make each block differ from all the others.
/// `None` for a block that no extension of up to a dozen lines tells apart.
pub fn extensions(file: &[&str], len: usize, starts: &[usize]) -> Vec<Option<Extension>> {
    let norm: Vec<String> = file.iter().map(|line| normal(line)).collect();
    starts
        .iter()
        .map(|&start| {
            for total in 1..=MAX_EXTENSION {
                for above in (0..=total).rev() {
                    let below = total - above;
                    if above > start || start + len + below > norm.len() {
                        continue;
                    }
                    let mine = &norm[start - above..start + len + below];
                    let unique = starts.iter().all(|&other| {
                        other == start
                            || other < above
                            || other + len + below > norm.len()
                            || &norm[other - above..other + len + below] != mine
                    });
                    if unique {
                        return Some(Extension { above, below });
                    }
                }
            }
            None
        })
        .collect()
}
