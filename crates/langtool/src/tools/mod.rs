//! The `langtool_*` tools, one file per tool named after it (`find_symbols.rs` is
//! `langtool_find_symbols`). Each file holds the tool's `DESCRIPTION` (what the agent is told),
//! its parameters and its handler.

pub mod find_references;
pub mod find_symbols;
pub mod read_file;
pub mod run_action;
pub mod summarize_codebase;
pub mod summarize_feature;
pub mod summarize_symbol;
mod summarize_symbol_story;
pub mod view_callgraph;

pub use find_references::FindReferencesParams;
pub use find_symbols::FindSymbolsParams;
pub use read_file::{ReadFileEntry, ReadFileParams};
pub use run_action::RunActionParams;
pub use summarize_codebase::SummarizeCodebaseParams;
pub use summarize_feature::SummarizeFeatureParams;
pub use summarize_symbol::SummarizeSymbolParams;
pub use view_callgraph::ViewCallgraphParams;
