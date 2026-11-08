mod common;
mod langtool {
    mod callgraph_origins;
    mod find_references;
    mod find_symbols;
    mod read_file;
    mod run_action;
    mod view_callgraph;
}

pub(crate) use common::LIB;
