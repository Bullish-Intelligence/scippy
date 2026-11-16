//! The output of `scippy index`: the text summary and the `--json` envelope.

use std::path::PathBuf;
use std::process::ExitCode;

use scippy_core::{Diagnostic, IndexReport, ScippyError};
use scippy_metalayer::{Publication, SnapshotFacts};
use serde_json::{Value, json};

use crate::metalayer::{CliError, run_envelope};

/// The exit code of a run that finished with lost input or an inexact analysis, without a
/// failure. `--strict` turns lost input into a failure (code 1) before this.
pub const EXIT_INCOMPLETE: u8 = 3;

/// The exit code of a run that finished: 0 when the run lost no input and its analysis is
/// exact, else [`EXIT_INCOMPLETE`].
pub fn exit_code(facts: &SnapshotFacts) -> ExitCode {
    if facts.input_complete && facts.analysis_exact {
        ExitCode::SUCCESS
    } else {
        ExitCode::from(EXIT_INCOMPLETE)
    }
}

/// Everything that one finished run reports.
pub struct RunOutput {
    pub report: IndexReport,
    pub output: PathBuf,
    pub publication: Option<Publication>,
}

impl RunOutput {
    /// The text of the human mode. Each line ends with a new line.
    pub fn text(&self) -> String {
        let mut text = format!("{}\n", self.report);
        if let Some(publication) = &self.publication {
            text.push_str(&format!("metalayer revision: {}\n", publication.revision));
            if let Some(r) = &publication.rebind {
                if r.skipped {
                    text.push_str("identity: unchanged\n");
                } else {
                    text.push_str(&format!(
                        "identity: bound {}, minted {}, ambiguous {}, orphaned {}, collected {}\n",
                        r.bound_total(),
                        r.minted,
                        r.ambiguous,
                        r.orphaned,
                        r.collected
                    ));
                }
            }
        }
        text
    }

    /// The envelope of the `--json` mode.
    ///
    /// It has the head of the facts envelope and its flags `input_complete` and
    /// `analysis_exact`. It has no `revision` block: the revision of a publication is in
    /// `value.metalayer`.
    pub fn envelope(&self, facts: &SnapshotFacts) -> Value {
        let r = &self.report;
        let lost: Vec<Value> = facts
            .lost_files
            .iter()
            .map(|f| {
                json!({
                    "path": f.path,
                    "status": f.status,
                    "reason": f.reason,
                    "pack": f.pack,
                    "real_path": f.real_path,
                    "sha256": f.sha256,
                })
            })
            .collect();
        let metalayer = self.publication.as_ref().map(|p| {
            let identity = p.rebind.as_ref().map(|i| {
                json!({
                    "skipped": i.skipped,
                    "bound": i.bound_total(),
                    "minted": i.minted,
                    "ambiguous": i.ambiguous,
                    "orphaned": i.orphaned,
                    "collected": i.collected,
                })
            });
            json!({
                "database": p.database.to_string_lossy(),
                "revision": p.revision,
                "identity": identity,
            })
        });
        let m = &r.multi_target;
        let b = &r.bounded_by_kind;
        let value = json!({
            "output": {
                "scip": self.output.to_string_lossy(),
            },
            "files": {
                "discovered": r.files_discovered,
                "indexed": r.files_indexed,
                "skipped": r.files_skipped,
                "shadowed": r.files_shadowed,
                "discovery_skipped": r.discovery_skipped,
            },
            "declarations": r.declarations,
            "occurrences": r.occurrences,
            "references": {
                "resolved": r.references_resolved,
                "external": r.references_external,
                "unresolved": r.references_unresolved,
                "ambiguous": r.references_ambiguous,
                "bounded": r.references_bounded,
                "bounded_by_kind": {
                    "work": b.work,
                    "queue_length": b.queue_length,
                    "arena_cells": b.arena_cells,
                    "failed": b.failed,
                },
                "unresolved_reasons": r.unresolved_reasons,
                "unjoined_end_kinds": r.unjoined_end_kinds,
                "multi_target": {
                    "raw": m.raw,
                    "shadow_filtered": m.shadow_filtered,
                    "pack_refused": m.pack_refused,
                    "nearest_binding": m.nearest_binding,
                    "alias_collapsed": m.alias_collapsed,
                    "builtin_shadow": m.builtin_shadow,
                    "same_symbol": m.same_symbol,
                    "ambiguous": m.ambiguous,
                    "unresolved": m.unresolved,
                    "bounded": m.bounded,
                },
            },
            "graph_cache": {
                "enabled": r.graph_cache_enabled,
                "hits": r.graph_cache_hits,
                "misses": r.graph_cache_misses,
                "failures": r.graph_cache_failures,
            },
            "lost_files": lost,
            "stale_files": r.stale_files,
            "diagnostics": r.diagnostics.iter().map(diagnostic).collect::<Vec<_>>(),
            "metalayer": metalayer,
        });
        run_envelope("index", facts.input_complete, facts.analysis_exact, value)
    }
}

fn diagnostic(d: &Diagnostic) -> Value {
    json!({
        "severity": format!("{:?}", d.severity).to_lowercase(),
        "stage": format!("{:?}", d.stage).to_lowercase(),
        "code": d.code,
        "file": d.file,
        "range": d.range,
        "message": d.message,
    })
}

/// The error envelope of a failed run.
pub fn error_envelope(error: ScippyError) -> Value {
    crate::metalayer::error_envelope("index", &CliError::from(error))
}
