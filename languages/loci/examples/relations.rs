//! Print the relations of a vault in the format of `loci --json graph/relations`.
//!
//! Usage: relations <vault root>
//! The output is one JSON record for each line, sorted by source and span.
//! The example reads the vault, its ignore list, and its roots. It writes nothing.

use std::path::Path;

use scippy_loci::export_workspace;

fn main() {
    let root = std::env::args().nth(1).expect("vault root");
    let mut records = match export_workspace(Path::new(&root)) {
        Ok(records) => records,
        Err(message) => {
            eprintln!("relations: {message}");
            std::process::exit(1);
        }
    };
    records.sort_by(|a, b| (&a.source, a.span).cmp(&(&b.source, b.span)));
    for r in records {
        println!("{}", r.to_json());
    }
}
