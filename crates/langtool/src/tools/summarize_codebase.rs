//! `langtool_summarize_codebase`: a one-call overview of a codebase or sub-tree with layers,
//! doors, a start path, the work front and a folder tree.

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};

use schemars::JsonSchema;
use serde::Deserialize;

use super::summarize_symbol::candidates;
use crate::common::{ItemKind, Result, ServerConfig, Workspace};
use crate::engine::lens::{
    codebase::{Codebase, span},
    facts::{FileFacts, facts as read_facts},
    graph::{Tag, in_test, is_named},
    layers::{Layers, Mentions},
    store::{FactStore, survey},
    words::Words,
    zoom::{Zoom, cut, fit, limit_lines},
};
use crate::engine::origins::Origins;
use crate::engine::scan::{ScanTarget, label_root};
use crate::language::{language_for_path, project_root, registry};

pub const DESCRIPTION: &str = "\
A compact overview of the code at `implicit_path` (default `.`): header, layers, doors, start \
path, work front and folder tree. The zoom (`low`/`medium`/`high`/`max`, default `medium`) grows \
the row budget (24/48/96/192) and the number of folder levels opened. Use it before reading \
individual files. Each collapsed row in the tree ends with the exact call to zoom into that \
branch: `> langtool_summarize_codebase implicit_path=…`.\
";

/// Parameters for `langtool_summarize_codebase`.
#[derive(Debug, Default, Deserialize, JsonSchema)]
pub struct SummarizeCodebaseParams {
    /// File or directory to overview, relative to the project root or absolute. Default `.`.
    #[serde(default)]
    pub implicit_path: Option<String>,
    /// How much to show: `low`, `medium` (default), `high` or `max`.
    #[serde(default)]
    pub zoom: Option<String>,
    /// Include dotfiles and gitignored files.
    #[serde(default)]
    pub show_gitignored: Option<bool>,
}

pub fn summarize_codebase(
    workspace: Option<&dyn Workspace>,
    cfg: &ServerConfig,
    store: &FactStore,
    p: SummarizeCodebaseParams,
) -> Result<String> {
    let (zoom, zoom_note) = Zoom::from_param(p.zoom.as_deref());
    let target = ScanTarget {
        path: p.implicit_path.as_deref(),
        max_depth: cfg.max_dir_depth,
        show_gitignored: p.show_gitignored,
    };

    // The shown paths are relative to the scanned folder when it lies outside the project, so the
    // manifests and the projects row are looked up there.
    let shown_from = label_root(workspace, &target)?;
    let root = Some(shown_from.as_path());
    let sv = survey(workspace, &target, store)?;
    if sv.files.is_empty() {
        let mut out = "no supported files found\n".to_string();
        if let Some(note) = zoom_note {
            out.push_str(&format!("{note}\n"));
        }
        return Ok(out);
    }

    let origins = root
        .map(|r| Origins::new(r, &[]))
        .unwrap_or_else(|| Origins::new(Path::new(""), &[]));
    let codebase = Codebase::new(sv.files, &origins);

    let labels: Vec<&str> = codebase.files.iter().map(|(l, _)| l.as_str()).collect();
    let project_roots = collect_project_roots(root, &labels);
    let links = collect_member_links(&project_roots);
    let file_edges = codebase.graph.file_edges();
    let scan_root = root.unwrap_or(Path::new(""));
    let tests: Vec<bool> = codebase
        .files
        .iter()
        .map(|(_, facts)| !facts.defs.is_empty() && facts.defs.iter().all(in_test))
        .collect();
    let mentions = Mentions {
        weights: &file_edges,
        names: codebase.graph.file_names(),
        tests: &tests,
    };
    let layers = Layers::compute(&labels, &mentions, scan_root, &links, &project_roots);

    let door_files = collect_door_files(&codebase, root, &labels);

    let per_file_names: Vec<Vec<String>> = codebase
        .files
        .iter()
        .map(|(_, facts)| {
            facts
                .defs
                .iter()
                .filter(|d| d.exported)
                .map(|d| d.symbol.name.clone())
                .collect()
        })
        .collect();
    let words = Words::new(&per_file_names);

    let mut out = String::new();

    // 1. Header.
    let total_lines: usize = codebase.files.iter().map(|(_, f)| f.lines).sum();
    let total_defs: usize = codebase.files.iter().map(|(_, f)| f.defs.len()).sum();
    let test_defs: usize = codebase
        .files
        .iter()
        .map(|(_, f)| f.defs.iter().filter(|d| d.symbol.test).count())
        .sum();
    let languages = detect_languages(&labels);
    let path_shown = p.implicit_path.as_deref().unwrap_or(".");
    let test_pct = if total_defs > 0 {
        test_defs * 100 / total_defs
    } else {
        0
    };
    out.push_str(&format!(
        "{path}  {lang}  {files} files  {lines} lines  {defs} defs ({test}% test)  zoom {zoom}\n",
        path = path_shown,
        lang = languages,
        files = codebase.files.len(),
        lines = total_lines,
        defs = total_defs,
        test = test_pct,
        zoom = zoom.name(),
    ));

    // 2. Projects (when multiple manifest roots).
    if project_roots.len() > 1 {
        out.push_str("projects:\n");
        let mut entries: Vec<(String, String)> = Vec::new();
        for pr in &project_roots {
            let rel = root
                .and_then(|r| pr.strip_prefix(r).ok())
                .map(|r| r.to_string_lossy().replace('\\', "/"))
                .unwrap_or_else(|| pr.to_string_lossy().to_string());
            if rel.is_empty() {
                // Files with no manifest above them fall back to the scan root: the header counts them.
                continue;
            }
            let pr_labels: Vec<&str> = labels
                .iter()
                .copied()
                .filter(|l| l.starts_with(rel.as_str()))
                .collect();
            let pr_files = pr_labels.len();
            let pr_lines: usize = pr_labels
                .iter()
                .filter_map(|l| {
                    codebase
                        .files
                        .iter()
                        .find(|(fl, _)| fl.as_str() == *l)
                        .map(|(_, f)| f.lines)
                })
                .sum();
            let pr_lang = detect_languages(&pr_labels);
            entries.push((
                rel,
                format!("{pr_lang}  {pr_files} files  {pr_lines} lines"),
            ));
        }
        push_projects(&mut out, &entries);
    }

    // 3. Layers.
    let layer_rows = layers.rows();
    if !layer_rows.is_empty() {
        out.push_str("layers: ");
        out.push_str(&layer_rows[0]);
        out.push('\n');
        for extra in &layer_rows[1..] {
            out.push_str(&format!("        {extra}\n"));
        }
    }

    // 4. Doors, at most half of the row budget so that the start path, the front and the tree
    // keep their rows in a folder of many projects.
    let door_rows = render_doors(&codebase, &labels, &door_files);
    if !door_rows.is_empty() {
        let total = door_rows.len();
        let shown = total.min((zoom.rows() / 2).max(1));
        let door_rows = spread(door_rows, shown);
        out.push_str("doors:  ");
        out.push_str(&door_rows[0]);
        out.push('\n');
        for row in &door_rows[1..] {
            out.push_str(&format!("        {row}\n"));
        }
        if total > shown {
            out.push_str(&format!("        … {} more doors\n", total - shown));
        }
    }

    // 5. Start, left out in a folder of several projects: a path that hops from one unrelated
    // project to the next shows no way into any of them (a scan of one project gives its own).
    let start_stops = if project_roots.len() > 1 {
        Vec::new()
    } else {
        render_start(&codebase, &layers)
    };
    if !start_stops.is_empty() {
        out.push_str("start:  ");
        out.push_str(&start_stops.join(" > "));
        out.push('\n');
    }

    // 6. Front, only when the scan and the registered project lie one in the other: the open work
    // of a project says nothing of a repository somewhere else, and its task words would hit
    // common names there (`main`).
    let mut feature_hint: Option<String> = None;
    let related =
        |ws: &&dyn Workspace| ws.root().starts_with(scan_root) || scan_root.starts_with(ws.root());
    if let Some(ws) = workspace.filter(related) {
        let front = ws.work_front();
        if !front.moved.is_empty() || !front.open.is_empty() {
            let mut front_lines: Vec<String> = Vec::new();
            let open_cap = (zoom.rows() / 12).max(2);
            if let Some(item) = front
                .open
                .iter()
                .find(|item| matches!(item.kind, ItemKind::Task))
            {
                let mut names: Vec<String> = Vec::new();
                for &node in named_by(&codebase, &item.text).iter().take(3) {
                    let name = codebase.def(node).symbol.name.clone();
                    if !names.contains(&name) {
                        names.push(name);
                    }
                }
                let words = task_words(&item.text);
                if !names.is_empty() {
                    let list: Vec<String> =
                        names.iter().map(|name| format!("\"{name}\"")).collect();
                    feature_hint = Some(format!(
                        "hint: > langtool_summarize_feature symbol=[{}] resolves task {}",
                        list.join(", "),
                        item.id
                    ));
                } else if !words.is_empty() {
                    feature_hint = Some(format!(
                        "hint: > langtool_summarize_feature query=\"{words}\" resolves task {}",
                        item.id
                    ));
                }
            }
            for item in front.open.iter().take(open_cap) {
                let kind = match item.kind {
                    ItemKind::Task => "task",
                    ItemKind::Note => "note",
                };
                let named = named_by(&codebase, &item.text);
                if named.is_empty() {
                    front_lines.push(fit(&format!("{kind} {}: {}", item.id, item.text)));
                } else {
                    let places: Vec<String> = named
                        .iter()
                        .take(3)
                        .map(|&node| place(&codebase, node))
                        .collect();
                    front_lines.push(fit(&format!("{kind} {} → {}", item.id, places.join(" · "))));
                }
            }
            if front.open.len() > open_cap {
                front_lines.push(format!("… {} more open items", front.open.len() - open_cap));
            }
            if !front.moved.is_empty() {
                for m in front.moved.iter().take(3) {
                    let who = if m.outside { "outside" } else { "you" };
                    let delta = symbol_delta(ws, &codebase, &m.path, &m.turn)
                        .map(|d| format!("  {d}"))
                        .unwrap_or_default();
                    front_lines.push(fit(&format!(
                        "moved: {} ({who}, {}){delta}",
                        m.path, m.turn
                    )));
                }
                if front.moved.len() > 3 {
                    front_lines.push(format!("moved: … {} more files", front.moved.len() - 3));
                }
            }
            if !front_lines.is_empty() {
                out.push_str("front:  ");
                out.push_str(&front_lines[0]);
                out.push('\n');
                for line in &front_lines[1..] {
                    out.push_str(&format!("        {line}\n"));
                }
            }
        }
    }

    // 7. Tree.
    let tree_rows = render_tree(
        &codebase,
        &door_files,
        &words,
        &labels,
        zoom,
        p.implicit_path.as_deref(),
    );
    for row in &tree_rows {
        out.push_str(row);
        out.push('\n');
    }

    // Ceiling under the section budgets: the header rows come on top of the tree's own budget.
    let mut out = limit_lines(&out, zoom.rows() + 24);

    // 8. Hint.
    if let Some(note) = zoom_note {
        out.push_str(&format!("{note}\n"));
    }
    if let Some(hint) = feature_hint {
        out.push_str(&format!("{hint}\n"));
    }
    if let Some(next) = zoom.next() {
        out.push_str(&format!("hint: use zoom={} for more detail\n", next.name()));
    }

    Ok(out)
}

// ── helpers ────────────────────────────────────────────────────────────────────

/// The listed definitions that an open item names: words in backticks and words that look like
/// identifiers (`snake_case`, `CamelCase`), four letters or more, that one to three definitions of
/// the survey bear. A lookup in the project's own definitions, with no list of words.
fn named_by(codebase: &Codebase, text: &str) -> Vec<usize> {
    let mut words: Vec<&str> = Vec::new();
    for (at, span) in text.split('`').enumerate() {
        let backticked = at % 2 == 1;
        for word in span.split(|c: char| !c.is_alphanumeric() && c != '_') {
            let identifier = word.contains('_') || word.chars().skip(1).any(char::is_uppercase);
            if word.chars().count() >= 4 && (backticked || identifier) && !words.contains(&word) {
                words.push(word);
            }
        }
    }
    let mut found: Vec<usize> = Vec::new();
    for word in words {
        let nodes: Vec<usize> = candidates(codebase, word, &[])
            .into_iter()
            .filter(|&node| codebase.listed(node))
            .collect();
        if (1..=3).contains(&nodes.len()) {
            for node in nodes {
                if !found.contains(&node) {
                    found.push(node);
                }
            }
        }
    }
    found
}

/// `name file.rs:line`, where a definition starts.
fn place(codebase: &Codebase, node: usize) -> String {
    let def = codebase.def(node);
    let label = codebase.label(codebase.file_of(node));
    let file = label.rsplit('/').next().unwrap_or(label);
    format!("{} {file}:{}", def.symbol.name, def.symbol.first)
}

/// The first few words of a task that can name code: four letters or more.
fn task_words(text: &str) -> String {
    text.split(|c: char| !c.is_alphanumeric() && c != '_')
        .filter(|word| word.chars().count() >= 4)
        .take(5)
        .collect::<Vec<_>>()
        .join(" ")
}

/// What a moved file gained (`+`), changed in its signature (`~`) and lost (`-`) in the turn that
/// last changed it, from the definitions before that turn and now. `None` when the language has no
/// syntax, the file is new, or nothing changed at the level of names and signatures.
fn symbol_delta(
    workspace: &dyn Workspace,
    codebase: &Codebase,
    path: &str,
    turn: &str,
) -> Option<String> {
    let old = workspace.read_before(turn, path).ok()??;
    let before = read_facts(registry::for_path(path)?, &old)?;
    let now = &codebase.files.iter().find(|(label, _)| label == path)?.1;
    let signatures = |file: &FileFacts| -> BTreeMap<String, String> {
        file.defs
            .iter()
            .filter(|def| is_named(def) && !in_test(def))
            .map(|def| (def.symbol.name.clone(), def.symbol.signature.clone()))
            .collect()
    };
    let (was, is) = (signatures(&before), signatures(now));
    let added: Vec<&str> = is
        .keys()
        .filter(|name| !was.contains_key(*name))
        .map(String::as_str)
        .collect();
    let changed: Vec<&str> = is
        .iter()
        .filter(|(name, sig)| was.get(*name).is_some_and(|old| old != *sig))
        .map(|(name, _)| name.as_str())
        .collect();
    let removed: Vec<&str> = was
        .keys()
        .filter(|name| !is.contains_key(*name))
        .map(String::as_str)
        .collect();
    let mut parts: Vec<String> = Vec::new();
    for (mark, names) in [('+', added), ('~', changed), ('-', removed)] {
        for name in names.iter().take(3) {
            parts.push(format!("{mark}{name}"));
        }
        if names.len() > 3 {
            parts.push(format!("{mark}…{}", names.len() - 3));
        }
    }
    (!parts.is_empty()).then(|| format!("Δ {}", parts.join(" ")))
}

/// The unique language names of the files, comma-separated.
fn detect_languages(labels: &[&str]) -> String {
    let mut seen: BTreeSet<&'static str> = BTreeSet::new();
    for label in labels {
        if let Some(lang) = language_for_path(label) {
            seen.insert(lang.name);
        }
    }
    if seen.is_empty() {
        return "unknown".to_string();
    }
    seen.into_iter().collect::<Vec<_>>().join(",")
}

/// The manifest roots found under the scan root, deduped and sorted. A root below a `test` or
/// `tests` folder is a fixture repository of the tests, not a project of its own.
fn collect_project_roots(root: Option<&Path>, labels: &[&str]) -> Vec<PathBuf> {
    let root = match root {
        Some(r) => r,
        None => return Vec::new(),
    };
    let mut roots: BTreeSet<PathBuf> = BTreeSet::new();
    for label in labels {
        let abs = root.join(label);
        if let Some(syntax) = registry::for_path(label) {
            let pr = project_root(syntax, &abs, root);
            let fixture = pr.strip_prefix(root).is_ok_and(|rel| {
                rel.components()
                    .any(|part| part.as_os_str() == "test" || part.as_os_str() == "tests")
            });
            if !fixture {
                roots.insert(pr);
            }
        }
    }
    roots.into_iter().collect()
}

/// File indices that are doors: graph doors plus manifest entrypoints.
fn collect_door_files(
    codebase: &Codebase,
    root: Option<&Path>,
    labels: &[&str],
) -> BTreeSet<usize> {
    // Graph doors.
    let mut doors: BTreeSet<usize> = codebase
        .file_tags
        .iter()
        .enumerate()
        .filter(|(file, tag)| **tag == Tag::Door && !codebase.is_test_file(*file))
        .map(|(i, _)| i)
        .collect();

    // Manifest entrypoints.
    let root = match root {
        Some(r) => r,
        None => return doors,
    };
    let project_roots = collect_project_roots(Some(root), labels);
    for pr in &project_roots {
        for syntax in registry::supported_languages().filter_map(|l| registry::of(l)) {
            for ep in syntax.entrypoints(pr) {
                if let Ok(rel) = ep.strip_prefix(root) {
                    let rel_str = rel.to_string_lossy().replace('\\', "/");
                    if let Some(idx) = labels.iter().position(|l| *l == rel_str)
                        && !codebase.is_test_file(idx)
                    {
                        doors.insert(idx);
                    }
                }
            }
        }
    }
    doors
}

/// The manifest member links declared by each project, for `Layers::compute`.
fn collect_member_links(project_roots: &[PathBuf]) -> Vec<(PathBuf, PathBuf)> {
    let mut links: Vec<(PathBuf, PathBuf)> = Vec::new();
    for project in project_roots {
        for syntax in registry::supported_languages().filter_map(|l| registry::of(l)) {
            links.extend(syntax.member_links(project));
        }
    }
    links.sort();
    links.dedup();
    links
}

/// Fewest sibling doors that fold into one row.
const FAMILY_MIN: usize = 4;
/// Most names of the other files that a folded row prints before it counts the rest.
const FAMILY_NAMES: usize = 6;

/// The signature of the file's top definition with its name blanked and any initializer cut off, so
/// that the `tools/*.rs` files, which each hold one `Params` struct or `DESCRIPTION`, share a shape.
fn door_shape(codebase: &Codebase, file: usize) -> Option<String> {
    let top = *codebase.headline(file).first()?;
    let symbol = &codebase.def(top).symbol;
    if symbol.name.is_empty() {
        return None;
    }
    let signature = symbol.signature.split('=').next().unwrap_or_default();
    Some(
        signature
            .replace(symbol.name.as_str(), "_")
            .trim()
            .to_string(),
    )
}

/// The file name of `label` without folders and extension.
fn file_stem(label: &str) -> &str {
    let name = label.rsplit('/').next().unwrap_or(label);
    name.rsplit_once('.').map_or(name, |(stem, _)| stem)
}

/// One row for a family of sibling doors: the folder, how many, their shape, the one with the most
/// dependents and the names of the others.
fn family_row(
    codebase: &Codebase,
    labels: &[&str],
    (dir, shape): (&str, &str),
    files: &[usize],
) -> String {
    let mut ranked = files.to_vec();
    ranked.sort_by(|&a, &b| {
        codebase
            .graph
            .file_load(b)
            .ca
            .total_cmp(&codebase.graph.file_load(a).ca)
    });
    let exemplar = ranked[0];
    let others: Vec<&str> = files
        .iter()
        .filter(|&&file| file != exemplar)
        .map(|&file| file_stem(labels[file]))
        .collect();
    let shown = others
        .iter()
        .take(FAMILY_NAMES)
        .copied()
        .collect::<Vec<_>>()
        .join(" ");
    let more = others.len().saturating_sub(FAMILY_NAMES);
    let dir = if dir.is_empty() { "." } else { dir };
    let mut row = format!(
        "{dir}/  {} files of one shape `{shape}`  most used {}  also {shown}",
        files.len(),
        file_stem(labels[exemplar])
    );
    if more > 0 {
        row.push_str(&format!(" +{more}"));
    }
    row
}

/// Fewest exported methods of one owner, none of them mentioned, that fold into one phrase.
const OWNER_MIN: usize = 4;

/// `Owner 28 methods, no callers` for the owner in `file` with the most exported members that
/// nothing mentions (handlers a macro registers, trait entries), or `None` below [`OWNER_MIN`].
/// The count comes from the graph alone: no name list, no check for the macro.
fn owner_fold(codebase: &Codebase, file: usize) -> Option<String> {
    let facts = codebase.facts(file);
    let mut counts: BTreeMap<&str, usize> = BTreeMap::new();
    for node in codebase.graph.nodes_of(file) {
        let def = codebase.def(node);
        let Some(parent) = def.parent else {
            continue;
        };
        if def.exported && is_named(def) && !in_test(def) && codebase.graph.load(node).ca <= 0.0 {
            *counts
                .entry(facts.defs[parent].symbol.name.as_str())
                .or_default() += 1;
        }
    }
    let (owner, count) = counts.into_iter().max_by_key(|&(_, count)| count)?;
    (count >= OWNER_MIN).then(|| format!("{owner} {count} methods, no callers"))
}

/// Render `doors:` rows, each with the top folder it belongs to. Four or more doors of one folder
/// whose top definitions have the same shape fold to one row that names the most used of them and
/// the others. A door file whose owner type has [`OWNER_MIN`] or more exported methods that
/// nothing calls says so in its row.
fn render_doors(
    codebase: &Codebase,
    labels: &[&str],
    door_files: &BTreeSet<usize>,
) -> Vec<(String, String)> {
    let mut families: std::collections::BTreeMap<(String, String), Vec<usize>> =
        std::collections::BTreeMap::new();
    for &file in door_files {
        if let Some(shape) = door_shape(codebase, file) {
            let dir = labels[file].rsplit_once('/').map_or("", |(dir, _)| dir);
            families
                .entry((dir.to_string(), shape))
                .or_default()
                .push(file);
        }
    }
    families.retain(|_, files| files.len() >= FAMILY_MIN);
    let folded: BTreeSet<usize> = families.values().flatten().copied().collect();

    let mut rows: Vec<(String, String)> = Vec::new();
    for &file in door_files {
        if folded.contains(&file) {
            continue;
        }
        let label = labels[file];
        let top_names: Vec<&str> = codebase
            .headline(file)
            .iter()
            .take(3)
            .map(|&node| codebase.def(node).symbol.name.as_str())
            .collect();
        let tag = codebase.file_tags[file].as_str();
        let mut row = label.to_string();
        if !tag.is_empty() {
            row.push_str(&format!("  {tag}"));
        }
        if let Some(owner) = owner_fold(codebase, file) {
            row.push_str(&format!("  {owner}"));
        }
        if !top_names.is_empty() {
            row.push_str(&format!("  {}", top_names.join(" ")));
        }
        let dir = label.rsplit_once('/').map_or("", |(dir, _)| dir);
        rows.push((first_segment(dir).to_string(), fit(&row)));
    }
    for ((dir, shape), files) in &families {
        rows.push((
            first_segment(dir).to_string(),
            fit(&family_row(codebase, labels, (dir, shape), files)),
        ));
    }
    rows
}

/// The first folder of a project-relative folder path (empty for the project root).
fn first_segment(dir: &str) -> &str {
    dir.split('/').next().unwrap_or("")
}

/// The `shown` door rows to print. All of them when they fit; otherwise one row of each top
/// folder in turn, so that a folder of many projects shows doors of every project instead of
/// the first project's. The rows keep their order.
fn spread(rows: Vec<(String, String)>, shown: usize) -> Vec<String> {
    if rows.len() <= shown {
        return rows.into_iter().map(|(_, row)| row).collect();
    }
    let mut by_folder: std::collections::BTreeMap<&str, Vec<usize>> =
        std::collections::BTreeMap::new();
    for (at, (folder, _)) in rows.iter().enumerate() {
        by_folder.entry(folder.as_str()).or_default().push(at);
    }
    let mut keep = vec![false; rows.len()];
    let mut left = shown;
    let mut round = 0;
    while left > 0 {
        let mut took = false;
        for &at in by_folder.values().filter_map(|list| list.get(round)) {
            if left == 0 {
                break;
            }
            keep[at] = true;
            left -= 1;
            took = true;
        }
        if !took {
            break;
        }
        round += 1;
    }
    rows.into_iter()
        .zip(keep)
        .filter_map(|((_, row), keep)| keep.then_some(row))
        .collect()
}

/// Print the `projects:` rows as (folder, details) pairs. Projects of one top folder that share
/// more folders print that shared path once, and each project below it by the rest of its path.
fn push_projects(out: &mut String, entries: &[(String, String)]) {
    let mut at = 0;
    while at < entries.len() {
        let top = first_segment(&entries[at].0);
        let end = entries[at..]
            .iter()
            .position(|(rel, _)| first_segment(rel) != top)
            .map_or(entries.len(), |n| at + n);
        let group = &entries[at..end];
        let shared = shared_folders(group);
        if group.len() > 1 && !shared.is_empty() {
            out.push_str(&format!("  {shared}/\n"));
            for (rel, details) in group {
                out.push_str(&format!("    {}  {details}\n", &rel[shared.len() + 1..]));
            }
        } else {
            for (rel, details) in group {
                out.push_str(&format!("  {rel}  {details}\n"));
            }
        }
        at = end;
    }
}

/// The longest folder path that every path of `group` lies below, by whole folders. Each path
/// keeps at least one folder of its own.
fn shared_folders(group: &[(String, String)]) -> String {
    let Some((first, _)) = group.first() else {
        return String::new();
    };
    let mut shared: Vec<&str> = first.split('/').collect();
    for (rel, _) in &group[1..] {
        let same = shared
            .iter()
            .zip(rel.split('/'))
            .take_while(|(a, b)| **a == *b)
            .count();
        shared.truncate(same);
    }
    let shortest = group
        .iter()
        .map(|(rel, _)| rel.split('/').count())
        .min()
        .unwrap_or(0);
    shared.truncate(shortest.saturating_sub(1));
    shared.join("/")
}

/// Render `start:` stops: up to 5 files sorted by layer then Ca, formatted as `file:line`.
fn render_start(codebase: &Codebase, layers: &Layers) -> Vec<String> {
    let n = codebase.files.len();
    if n == 0 {
        return Vec::new();
    }
    // Sort by Ca descending to find high-value files.
    let mut by_ca: Vec<usize> = (0..n).collect();
    by_ca.sort_by(|&a, &b| {
        codebase
            .graph
            .file_load(b)
            .ca
            .total_cmp(&codebase.graph.file_load(a).ca)
    });

    let candidates: Vec<usize> = by_ca
        .into_iter()
        .filter(|&f| !codebase.headline(f).is_empty())
        .take(5)
        .collect();

    if candidates.is_empty() {
        return Vec::new();
    }

    // Order by layer (foundation first).
    let mut ordered = candidates;
    ordered.sort_by_key(|&f| layers.layer[layers.of_file[f]]);

    ordered
        .iter()
        .filter_map(|&file| {
            let top = *codebase.headline(file).first()?;
            let def = codebase.def(top);
            Some(span(
                codebase.label(file),
                def.symbol.first,
                def.symbol.last,
            ))
        })
        .collect()
}

/// Render the flat file tree rows within `zoom.rows()` budget.
fn render_tree(
    codebase: &Codebase,
    door_files: &BTreeSet<usize>,
    words: &Words,
    _labels: &[&str],
    zoom: Zoom,
    implicit_path: Option<&str>,
) -> Vec<String> {
    let budget = zoom.rows();
    let mut all_rows: Vec<String> = Vec::new();

    for (file, (label, facts)) in codebase.files.iter().enumerate() {
        let raw_tag = codebase.file_tags[file];
        let effective_tag = if door_files.contains(&file) && raw_tag == Tag::Detail {
            Tag::Door
        } else {
            raw_tag
        };
        let tag_str = effective_tag.as_str();

        // Gist: file doc line else top TF-IDF words.
        let gist = facts.doc.clone().unwrap_or_else(|| {
            let names: Vec<String> = facts
                .defs
                .iter()
                .filter(|d| d.exported)
                .map(|d| d.symbol.name.clone())
                .collect();
            words.top(&names, 3).join(" ")
        });

        // Top 3 names.
        let top_names: Vec<&str> = codebase
            .headline(file)
            .iter()
            .take(3)
            .map(|&node| codebase.def(node).symbol.name.as_str())
            .collect();

        let gist_part = cut(&gist, 36);
        let row = fit(&format!(
            "{label}  {tag}{sep}{gist}  {names}",
            label = label,
            tag = tag_str,
            sep = if tag_str.is_empty() { "" } else { "  " },
            gist = gist_part,
            names = top_names.join(" "),
        ));
        all_rows.push(row);
    }

    if all_rows.len() <= budget {
        return all_rows;
    }

    let shown = budget.saturating_sub(1);
    let mut rows: Vec<String> = all_rows[..shown].to_vec();
    let hidden = all_rows.len() - shown;
    let top_hidden_names: Vec<&str> = (shown..all_rows.len())
        .take(5)
        .flat_map(|fi| {
            codebase
                .headline(fi)
                .into_iter()
                .take(1)
                .map(|node| codebase.def(node).symbol.name.as_str())
                .collect::<Vec<_>>()
        })
        .collect();
    let path_arg = implicit_path.unwrap_or(".");
    let next_zoom = zoom.next().map(|z| z.name()).unwrap_or("max");
    rows.push(fit(&format!(
        "+{hidden} files: {names}  > langtool_summarize_codebase implicit_path={path_arg} zoom={next_zoom}",
        names = top_hidden_names.join(" "),
    )));
    rows
}
