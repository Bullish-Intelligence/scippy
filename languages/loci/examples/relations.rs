//! Print the relations of a vault in the format of `loci --json graph/relations`.
//!
//! Usage: relations <vault root>
//! The output is one JSON record for each line, sorted by source and span.

use std::fs;
use std::path::Path;

use scippy_loci::{Settings, export_relations, list_files};

fn main() {
    let root = std::env::args().nth(1).expect("vault root");
    let root = Path::new(&root);
    let settings = Settings::read(root)
        .expect("manifest")
        .expect(".loci/vault.toml");
    let files = list_files(root);
    let mut records = export_relations(&settings, &files, |path| fs::read(root.join(path)).ok());
    records.sort_by(|a, b| (&a.source, a.span).cmp(&(&b.source, b.span)));
    for r in records {
        println!("{}", r.to_json());
    }
}
