//! Reference source for the contextgraph-treesitter provider. The line-based
//! extractor turns the definitions below into Symbol frames and a Graph frame.

use std::collections::HashMap;

pub struct Config {
    pub name: String,
    pub settings: HashMap<String, String>,
}

pub fn parse_config(text: &str) -> Config {
    let name = normalize(text);
    Config {
        name,
        settings: HashMap::new(),
    }
}

fn normalize(text: &str) -> String {
    text.trim().to_lowercase()
}
