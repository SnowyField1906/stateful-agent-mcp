mod common;
mod langtool {
    mod find_references;
    mod find_symbols;
    mod read_file;
    mod run_action;
    mod steering;
    mod view_callgraph;
}

pub(crate) const LIB: &str = concat!(
    "pub struct Store;\n",
    "impl Store {\n",
    "    pub fn new() -> Store { Store }\n",
    "    pub fn put(\n",
    "        &self,\n",
    "        key: &str,\n",
    "    ) -> Option<String> {\n",
    "        let v = Some(key.to_string());\n",
    "        v.map(|s| s)\n",
    "    }\n",
    "}\n",
    "pub fn open() -> Store { Store::new() }\n",
    "pub fn setup() { let s = open(); s.put(\"k\"); }\n",
);
