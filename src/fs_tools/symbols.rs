//! `code_symbols`: definitions in a file or below a directory, for languages with a
//! [`Syntax`](crate::lang::Syntax). Output, grouped by file:
//! ```text
//! src/a.rs
//! 5..9\timpl P\timpl P
//! 6..8\tfn new (in impl P)\tfn new() -> Self
//! ```

use std::collections::BTreeMap;

use crate::config::ServerConfig;
use crate::error::{AgentError, Result};
use crate::lang::LANGUAGES;
use crate::lang::symbols::{self, Symbol};
use crate::params::CodeSymbolsParams;
use crate::state::Project;

use super::scan::{ScanTarget, scan};

pub fn symbols(project: Option<&Project>, cfg: &ServerConfig, p: CodeSymbolsParams) -> Result<String> {
    let query = p.query.as_deref().map(str::trim).filter(|q| !q.is_empty());
    let kinds_filter: Option<BTreeMap<String, Vec<String>>> = match p.kinds {
        None => None,
        Some(raw) if raw.is_empty() => None,
        Some(raw) => {
            let mut map: BTreeMap<String, Vec<String>> = BTreeMap::new();
            for (lang, list) in raw {
                let key = lang.trim().to_ascii_lowercase();
                if !LANGUAGES.iter().any(|l| l.name == key && l.syntax.is_some()) {
                    return Err(AgentError::invalid(
                        "kinds",
                        format!("unsupported language `{lang}`; supported: rust, python"),
                    ));
                }
                let cleaned: Vec<String> = list
                    .into_iter()
                    .map(|k| k.trim().to_string())
                    .filter(|k| !k.is_empty())
                    .collect();
                map.entry(key).or_default().extend(cleaned);
            }
            Some(map)
        }
    };
    let target = ScanTarget {
        implicit_path: p.implicit_path.as_deref(),
        max_depth: cfg.max_dir_depth,
        show_gitignored: p.show_gitignored,
    };
    let found = scan(project, &target, |lang, syntax, text| {
        if let Some(map) = &kinds_filter {
            let Some(allowed) = map.get(lang.name) else {
                return Some(Vec::new());
            };
            if allowed.is_empty() {
                return Some(Vec::new());
            }
            let all = symbols::symbols(syntax, text)?;
            return Some(
                all.into_iter()
                    .filter(|s| {
                        symbols::matches(s, query, None)
                            && allowed.iter().any(|k| symbols::matches(s, None, Some(k)))
                    })
                    .collect::<Vec<Symbol>>(),
            );
        }
        let all = symbols::symbols(syntax, text)?;
        Some(
            all.into_iter()
                .filter(|s| symbols::matches(s, query, None))
                .collect::<Vec<Symbol>>(),
        )
    })?;
    found.render(cfg, &p.page, "no symbols", |group| symbols::render(group))
}
