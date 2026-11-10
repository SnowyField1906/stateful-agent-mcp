//! The mention graph weighs a mention by the rarity of the name, and reads the roles from the
//! loads.

use std::sync::Arc;

use stateful_agent_langtool::engine::lens::facts::{FileFacts, facts};
use stateful_agent_langtool::engine::lens::graph::{DefId, Graph, Tag};
use stateful_agent_language::registry;

type Files = Vec<(String, Arc<FileFacts>)>;

fn survey(sources: &[(&str, &str)]) -> Files {
    sources
        .iter()
        .map(|(name, source)| {
            let parsed = facts(registry::for_path(name).unwrap(), source).unwrap();
            (name.to_string(), Arc::new(parsed))
        })
        .collect()
}

/// The project number of each file when all of them lie in one project.
fn one_project(files: &Files) -> Vec<usize> {
    vec![0; files.len()]
}

fn graph(files: &Files) -> Graph {
    Graph::build(files, &one_project(files), &|_, _| false)
}

/// The node of the definition `name` in the file called `file`.
fn node(files: &Files, graph: &Graph, file: &str, name: &str) -> usize {
    let at = files.iter().position(|(label, _)| label == file).unwrap();
    let def = files[at]
        .1
        .defs
        .iter()
        .position(|def| def.symbol.name == name)
        .unwrap_or_else(|| panic!("no `{name}` in {file}"));
    graph.node(DefId { file: at, def })
}

fn near(left: f64, right: f64) -> bool {
    (left - right).abs() < 1e-9
}

#[test]
fn a_name_defined_once_is_a_full_edge() {
    let files = survey(&[
        ("a.rs", "pub fn parse() {}\n"),
        ("b.rs", "pub fn run() {\n    parse();\n}\n"),
    ]);
    let graph = graph(&files);
    let parse = node(&files, &graph, "a.rs", "parse");
    let run = node(&files, &graph, "b.rs", "run");
    let edges = graph.uses(run);
    assert_eq!(edges.len(), 1);
    assert_eq!(edges[0].to, parse);
    assert!(near(edges[0].weight, 1.0));
    assert_eq!(graph.used_by(parse)[0].to, run);
    assert!(near(graph.load(parse).ca, 1.0));
    assert!(near(graph.load(run).ce, 1.0));
    assert!(near(graph.file_load(0).ca, 1.0));
    assert!(near(graph.file_load(1).ce, 1.0));
    assert_eq!(
        graph.file_edges().into_iter().collect::<Vec<_>>(),
        [((1, 0), 1.0)]
    );
}

#[test]
fn a_name_defined_twice_gives_each_definition_half() {
    let files = survey(&[
        ("a1.rs", "pub fn go() {}\n"),
        ("a2.rs", "pub fn go() {}\n"),
        ("b.rs", "pub fn run() {\n    go();\n}\n"),
    ]);
    let graph = graph(&files);
    let run = node(&files, &graph, "b.rs", "run");
    let edges = graph.uses(run);
    assert_eq!(edges.len(), 2);
    assert!(edges.iter().all(|edge| near(edge.weight, 0.5)));
}

#[test]
fn a_name_defined_six_times_is_dropped() {
    let mut sources: Vec<(String, String)> = (0..6)
        .map(|i| (format!("n{i}.rs"), "pub fn new() {}\n".to_string()))
        .collect();
    sources.push(("b.rs".into(), "pub fn run() {\n    new();\n}\n".into()));
    let borrowed: Vec<(&str, &str)> = sources
        .iter()
        .map(|(name, source)| (name.as_str(), source.as_str()))
        .collect();
    let files = survey(&borrowed);
    let graph = graph(&files);
    let run = node(&files, &graph, "b.rs", "run");
    assert!(graph.uses(run).is_empty());
}

#[test]
fn a_name_the_file_defines_itself_means_its_own_definition() {
    let files = survey(&[
        ("a.rs", "pub fn parse() {}\n"),
        ("b.rs", "fn parse() {}\n\npub fn run() {\n    parse();\n}\n"),
    ]);
    let graph = graph(&files);
    let run = node(&files, &graph, "b.rs", "run");
    let own = node(&files, &graph, "b.rs", "parse");
    let edges = graph.uses(run);
    assert_eq!(edges.len(), 1);
    assert_eq!(edges[0].to, own);
    assert!(near(edges[0].weight, 1.0));
    assert!(graph.file_edges().is_empty());
}

#[test]
fn a_name_an_import_binds_outside_the_project_is_dropped() {
    let files = survey(&[
        ("a.rs", "pub fn parse() {}\n"),
        ("b.rs", "pub fn run() {\n    parse();\n}\n"),
    ]);
    let graph = Graph::build(&files, &one_project(&files), &|file, name| {
        file == 1 && name == "parse"
    });
    let run = node(&files, &graph, "b.rs", "run");
    assert!(graph.uses(run).is_empty());
}

#[test]
fn a_mention_never_crosses_projects() {
    let files = survey(&[
        ("a.rs", "pub fn parse() {}\n"),
        ("b.rs", "pub fn run() {\n    parse();\n}\n"),
    ]);
    let apart = Graph::build(&files, &[0, 1], &|_, _| false);
    let run = node(&files, &apart, "b.rs", "run");
    assert!(apart.uses(run).is_empty());
    assert!(apart.file_edges().is_empty());
    let together = Graph::build(&files, &[0, 0], &|_, _| false);
    assert_eq!(together.uses(run).len(), 1);
}

#[test]
fn a_name_is_counted_as_common_within_its_project_only() {
    // Six definitions of `parse` in other projects do not make the name common in project 0.
    let mut sources = vec![
        ("a.rs".to_string(), "pub fn parse() {}\n".to_string()),
        (
            "user.rs".to_string(),
            "pub fn run() {\n    parse();\n}\n".to_string(),
        ),
    ];
    for at in 0..6 {
        sources.push((format!("other{at}.rs"), "pub fn parse() {}\n".to_string()));
    }
    let refs: Vec<(&str, &str)> = sources
        .iter()
        .map(|(name, text)| (name.as_str(), text.as_str()))
        .collect();
    let files = survey(&refs);
    let mut projects = vec![0, 0];
    projects.extend((1..=6).collect::<Vec<usize>>());
    let graph = Graph::build(&files, &projects, &|_, _| false);
    let run = node(&files, &graph, "user.rs", "run");
    let edges = graph.uses(run);
    assert_eq!(edges.len(), 1);
    assert!(near(edges[0].weight, 1.0));
}

#[test]
fn a_mention_inside_test_code_counts_a_quarter() {
    let files = survey(&[
        ("a.rs", "pub fn parse() {}\n"),
        (
            "b.rs",
            "#[cfg(test)]\nmod tests {\n    fn check() {\n        parse();\n    }\n}\n",
        ),
    ]);
    let graph = graph(&files);
    let check = node(&files, &graph, "b.rs", "check");
    let edges = graph.uses(check);
    assert_eq!(edges.len(), 1);
    assert!(near(edges[0].weight, 0.25));
}

#[test]
fn an_impl_block_is_no_target_and_a_name_counts_once_per_definition() {
    let files = survey(&[(
        "a.rs",
        "pub struct Store;\n\nimpl Store {\n    pub fn put(&self) -> Store {\n        Store\n    }\n}\n",
    )]);
    let graph = graph(&files);
    let put = node(&files, &graph, "a.rs", "put");
    let store = node(&files, &graph, "a.rs", "Store");
    let block = files[0]
        .1
        .defs
        .iter()
        .position(|def| def.symbol.kind.name == "impl")
        .map(|def| graph.node(DefId { file: 0, def }))
        .unwrap();
    // `put` writes `Store` as a type and as a value: one name, one full edge.
    let edges = graph.uses(put);
    assert_eq!(edges.len(), 1);
    assert_eq!(edges[0].to, store);
    assert!(near(edges[0].weight, 1.0));
    // The block mentions the type it is written for; the block itself defines no name, so
    // nothing mentions it.
    let block_edges: Vec<usize> = graph.uses(block).iter().map(|e| e.to).collect();
    assert_eq!(block_edges, [store]);
    assert!(graph.used_by(block).is_empty());
}

#[test]
fn roles_are_read_from_the_loads() {
    let files = survey(&[
        ("top.rs", "pub fn top() {\n    mid();\n}\n"),
        (
            "mid.rs",
            "pub fn mid() {\n    leaf_a();\n    leaf_b();\n    leaf_c();\n}\n",
        ),
        (
            "leaf.rs",
            "pub fn leaf_a() {}\npub fn leaf_b() {}\npub fn leaf_c() {}\n",
        ),
    ]);
    let graph = graph(&files);
    let tags = graph.def_tags();
    let tag = |file: &str, name: &str| tags[node(&files, &graph, file, name)];
    assert_eq!(tag("top.rs", "top"), Tag::Door);
    assert_eq!(tag("mid.rs", "mid"), Tag::Hub);
    assert_eq!(tag("leaf.rs", "leaf_a"), Tag::Base);
    assert_eq!(graph.file_tags(), [Tag::Door, Tag::Hub, Tag::Base]);
}
