//! `scippy metalayer check`: is the index fresh.
//!
//! The command compares each manifest file that has a stored hash with the disk. It does the
//! same for the support files, such as `pyproject.toml`. It can also
//! compare the SCIP file (`--scip`) and look for new files (`--discover`). It exits 3 when the
//! index is stale. It ignores `--limit`: a list of this value is never cut.

use std::collections::BTreeSet;
use std::fmt::Write;
use std::path::PathBuf;
use std::process::ExitCode;

use clap::Args as ClapArgs;
use scippy_core::{IndexRequest, LanguagePack, ScippyError, discover_files};
use scippy_loci::LociPack;
use scippy_metalayer::{FileState, compare_file};
use scippy_python::PythonPack;
use scippy_typescript::TypeScriptPack;
use serde_json::json;

use super::common::{CliError, Common, Output, execute};

/// Check whether the index is fresh.
///
/// Exit code 0: fresh. Exit code 3: stale. The command ignores `--limit`.
#[derive(ClapArgs, Debug)]
pub struct Args {
    #[command(flatten)]
    common: Common,
    /// The project root to compare with. The default is the root that the database stores.
    #[arg(long, value_name = "PATH")]
    root: Option<PathBuf>,
    /// Compare this SCIP file with the size and the SHA-256 of the revision.
    #[arg(long, value_name = "PATH")]
    scip: Option<PathBuf>,
    /// Look for files that the walk accepts and the manifest lacks.
    #[arg(long)]
    discover: bool,
}

/// The built-in packs. Discovery uses those that the manifest names.
fn registry() -> [&'static dyn LanguagePack; 3] {
    [&PythonPack, &TypeScriptPack, &LociPack]
}

fn scip_state(state: FileState) -> &'static str {
    match state {
        FileState::Ok => "ok",
        FileState::Missing => "missing",
        _ => "changed",
    }
}

pub fn run(args: Args) -> ExitCode {
    execute("check", &args.common, |view| {
        let root = match &args.root {
            Some(root) => root.clone(),
            None => PathBuf::from(view.root_set()?.project_root),
        };
        let files = view.files(None)?;
        let (mut ok, mut unchecked) = (0_usize, 0_usize);
        let (mut changed, mut missing, mut stale) = (Vec::new(), Vec::new(), Vec::new());
        for file in &files {
            // The record carries the stale flag of the run. A stale file is also checked now.
            if file.stale {
                stale.push(file.path.clone());
            }
            match view.file_state(&root, file)? {
                FileState::Ok => ok += 1,
                FileState::Changed => changed.push(file.path.clone()),
                FileState::Missing => missing.push(file.path.clone()),
                _ => unchecked += 1,
            }
        }

        // The support files shape the facts, so a change in one makes the index stale. The
        // paths go to the same lists as the files: a client reads one set of lists.
        for input in view.support_inputs()? {
            match view.input_state(&root, &input)? {
                FileState::Changed => changed.push(input.path),
                FileState::Missing => missing.push(input.path),
                _ => {}
            }
        }
        changed.sort();
        missing.sort();

        let mut new = Vec::new();
        if args.discover {
            let packs: BTreeSet<&str> = files.iter().filter_map(|f| f.pack.as_deref()).collect();
            let languages: Vec<&dyn LanguagePack> = registry()
                .into_iter()
                .filter(|p| packs.contains(p.id()))
                .collect();
            let request = IndexRequest::new(
                root.clone(),
                args.scip.clone().unwrap_or_else(|| args.common.db.clone()),
            );
            let known: BTreeSet<&str> = files.iter().map(|f| f.path.as_str()).collect();
            let found = discover_files(&request, &languages).map_err(|e| match e {
                ScippyError::InvalidRoot { .. } => CliError::bad_argument(e.to_string()),
                e => CliError::from(e),
            })?;
            new = found
                .iter()
                .map(|p| p.as_str())
                .filter(|p| !known.contains(p))
                .map(str::to_owned)
                .collect();
            new.sort();
        }

        let scip = match &args.scip {
            None => "not_checked",
            Some(path) => {
                let revision = view.revision();
                scip_state(compare_file(
                    path,
                    Some(revision.scip_size),
                    &revision.scip_sha256,
                )?)
            }
        };
        let fresh = changed.is_empty()
            && missing.is_empty()
            && stale.is_empty()
            && new.is_empty()
            && scip != "changed"
            && scip != "missing";

        let mut text = String::new();
        let _ = writeln!(text, "{}", if fresh { "fresh" } else { "stale" });
        let _ = writeln!(
            text,
            "ok {ok}  changed {}  missing {}  stale {}  new {}  unchecked {unchecked}",
            changed.len(),
            missing.len(),
            stale.len(),
            new.len(),
        );
        let _ = writeln!(text, "scip  {scip}");
        for (state, paths) in [
            ("changed", &changed),
            ("missing", &missing),
            ("stale", &stale),
            ("new", &new),
        ] {
            for path in paths {
                let _ = writeln!(text, "{state}  {path}");
            }
        }

        let value = json!({
            "fresh": fresh,
            "counts": {
                "ok": ok,
                "changed": changed.len(),
                "missing": missing.len(),
                "stale": stale.len(),
                "new": new.len(),
                "unchecked": unchecked,
            },
            "changed": changed,
            "missing": missing,
            "stale": stale,
            "new": new,
            "scip": {"state": scip},
        });
        let mut output = Output::new(value, text);
        if !fresh {
            output.exit = 3;
        }
        Ok(output)
    })
}
