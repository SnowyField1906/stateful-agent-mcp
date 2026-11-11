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

fn survey_owned(sources: &[(String, String)]) -> Files {
    let borrowed: Vec<(&str, &str)> = sources
        .iter()
        .map(|(name, source)| (name.as_str(), source.as_str()))
        .collect();
    survey(&borrowed)
}

/// The nodes that `node` mentions.
fn targets(graph: &Graph, node: usize) -> Vec<usize> {
    graph.uses(node).iter().map(|edge| edge.to).collect()
}

/// `count` files `t0.rs`, `t1.rs`, ... each declaring a type `T0`, `T1`, ... with a `parse` method.
fn typed_parsers(count: usize) -> Vec<(String, String)> {
    (0..count)
        .map(|at| {
            (
                format!("t{at}.rs"),
                format!("pub struct T{at};\n\nimpl T{at} {{\n    pub fn parse(&self) {{}}\n}}\n"),
            )
        })
        .collect()
}

/// `count` files `m0.rs`, `m1.rs`, ... each with a free function `name`.
fn free_functions(name: &str, count: usize) -> Vec<(String, String)> {
    (0..count)
        .map(|at| (format!("m{at}.rs"), format!("pub fn {name}() {{}}\n")))
        .collect()
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
fn a_method_of_the_enclosing_type_is_found_among_many_of_the_same_name() {
    let mut sources = typed_parsers(7);
    sources.push((
        "caller.rs".into(),
        "impl T3 {\n    pub fn run(&self) {\n        self.parse();\n    }\n}\n".into(),
    ));
    let files = survey_owned(&sources);
    let graph = graph(&files);
    let run = node(&files, &graph, "caller.rs", "run");
    let own = node(&files, &graph, "t3.rs", "parse");
    let edges = graph.uses(run);
    assert_eq!(edges.len(), 1);
    assert_eq!(edges[0].to, own);
    assert!(near(edges[0].weight, 1.0));
}

#[test]
fn a_path_through_a_type_name_reaches_the_methods_of_that_type() {
    let mut sources = typed_parsers(7);
    sources.push((
        "caller.rs".into(),
        "pub fn run(t: &T3) {\n    T3::parse(t);\n}\n".into(),
    ));
    let files = survey_owned(&sources);
    let graph = graph(&files);
    let run = node(&files, &graph, "caller.rs", "run");
    let found = targets(&graph, run);
    assert!(found.contains(&node(&files, &graph, "t3.rs", "parse")));
    for other in [0, 1, 2, 4, 5, 6] {
        let parse = node(&files, &graph, &format!("t{other}.rs"), "parse");
        assert!(!found.contains(&parse), "T{other}::parse");
    }
}

#[test]
fn a_method_on_a_receiver_of_known_type_reaches_the_methods_of_that_type() {
    let mut sources = typed_parsers(7);
    sources.push((
        "caller.rs".into(),
        "pub fn run(t: &T3) {\n    t.parse();\n}\n".into(),
    ));
    let files = survey_owned(&sources);
    let graph = graph(&files);
    let run = node(&files, &graph, "caller.rs", "run");
    let found = targets(&graph, run);
    assert!(found.contains(&node(&files, &graph, "t3.rs", "parse")));
    assert!(!found.contains(&node(&files, &graph, "t0.rs", "parse")));
}

#[test]
fn a_method_on_a_trait_object_reaches_the_trait_and_its_implementations() {
    let mut sources = typed_parsers(6);
    sources.push((
        "p.rs".into(),
        "pub trait Parser {\n    fn parse(&self);\n}\n".into(),
    ));
    for name in ["a", "b"] {
        let ty = name.to_uppercase();
        sources.push((
            format!("{name}.rs"),
            format!("pub struct {ty};\n\nimpl Parser for {ty} {{\n    fn parse(&self) {{}}\n}}\n"),
        ));
    }
    sources.push((
        "caller.rs".into(),
        "pub fn run(p: &dyn Parser) {\n    p.parse();\n}\n".into(),
    ));
    let files = survey_owned(&sources);
    let graph = graph(&files);
    let run = node(&files, &graph, "caller.rs", "run");
    let found = targets(&graph, run);
    for name in ["a", "b"] {
        let parse = node(&files, &graph, &format!("{name}.rs"), "parse");
        assert!(found.contains(&parse), "{name}.rs");
    }
    for other in 0..6 {
        let parse = node(&files, &graph, &format!("t{other}.rs"), "parse");
        assert!(!found.contains(&parse), "T{other}::parse");
    }
}

#[test]
fn a_path_through_a_module_reaches_the_functions_of_that_module() {
    let mut sources = free_functions("parse", 6);
    sources.push(("other.rs".into(), "pub fn parse() {}\n".into()));
    sources.push((
        "caller.rs".into(),
        "pub fn run() {\n    other::parse();\n}\n".into(),
    ));
    let files = survey_owned(&sources);
    let graph = graph(&files);
    let run = node(&files, &graph, "caller.rs", "run");
    let edges = graph.uses(run);
    assert_eq!(edges.len(), 1);
    assert_eq!(edges[0].to, node(&files, &graph, "other.rs", "parse"));
    assert!(near(edges[0].weight, 1.0));
}

#[test]
fn an_import_from_a_module_reaches_the_functions_of_that_module() {
    let mut sources = free_functions("parse", 6);
    sources.push(("other.rs".into(), "pub fn parse() {}\n".into()));
    sources.push((
        "caller.rs".into(),
        "use crate::other::parse;\n\npub fn run() {\n    parse();\n}\n".into(),
    ));
    let files = survey_owned(&sources);
    let graph = graph(&files);
    let run = node(&files, &graph, "caller.rs", "run");
    let edges = graph.uses(run);
    assert_eq!(edges.len(), 1);
    assert_eq!(edges[0].to, node(&files, &graph, "other.rs", "parse"));
    assert!(near(edges[0].weight, 1.0));
}

#[test]
fn a_wrapper_that_delegates_to_another_module_keeps_that_edge() {
    let files = survey(&[
        ("other.rs", "pub fn parse() {}\n"),
        (
            "wrapper.rs",
            "use crate::other::parse as other_parse;\n\npub fn parse() {\n    other_parse();\n}\n",
        ),
    ]);
    let graph = graph(&files);
    let wrapper = node(&files, &graph, "wrapper.rs", "parse");
    let edges = graph.uses(wrapper);
    assert_eq!(edges.len(), 1);
    assert_eq!(edges[0].to, node(&files, &graph, "other.rs", "parse"));
    assert!(near(edges[0].weight, 1.0));
}

#[test]
fn a_qualifier_that_no_definition_fits_leaves_the_name_to_the_other_rules() {
    let files = survey(&[
        ("a.rs", "pub fn parse() {}\n"),
        ("b.rs", "pub fn run() {\n    Missing::parse();\n}\n"),
    ]);
    let graph = graph(&files);
    let run = node(&files, &graph, "b.rs", "run");
    let edges = graph.uses(run);
    assert_eq!(edges.len(), 1);
    assert_eq!(edges[0].to, node(&files, &graph, "a.rs", "parse"));
}

#[test]
fn a_path_through_an_import_of_a_dependency_is_not_a_project_module() {
    let mut sources = free_functions("read", 6);
    sources.push(("fs.rs".into(), "pub fn read() {}\n".into()));
    sources.push((
        "caller.rs".into(),
        "use std::fs;\n\npub fn run() {\n    fs::read();\n}\n".into(),
    ));
    let files = survey_owned(&sources);
    let caller = files
        .iter()
        .position(|(label, _)| label == "caller.rs")
        .unwrap();
    let graph = Graph::build(&files, &one_project(&files), &|file, name| {
        file == caller && name == "fs"
    });
    let run = node(&files, &graph, "caller.rs", "run");
    assert!(graph.uses(run).is_empty());
}

#[test]
fn a_narrowed_set_of_six_or_more_is_dropped_like_any_common_name() {
    // `Dyn` reaches the trait and every implementation: seven definitions of `parse`.
    let mut sources = vec![(
        "p.rs".to_string(),
        "pub trait Parser {\n    fn parse(&self);\n}\n".to_string(),
    )];
    for at in 0..6 {
        sources.push((
            format!("i{at}.rs"),
            format!(
                "pub struct I{at};\n\nimpl Parser for I{at} {{\n    fn parse(&self) {{}}\n}}\n"
            ),
        ));
    }
    sources.push((
        "caller.rs".into(),
        "pub fn run(p: &dyn Parser) {\n    p.parse();\n}\n".into(),
    ));
    let files = survey_owned(&sources);
    let graph = graph(&files);
    let run = node(&files, &graph, "caller.rs", "run");
    let named = |to: usize| {
        let id = graph.id(to);
        files[id.file].1.defs[id.def].symbol.name.clone()
    };
    // The type of the receiver stays a mention; none of the seven `parse` methods does.
    assert!(
        targets(&graph, run)
            .into_iter()
            .all(|to| named(to) != "parse")
    );
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
