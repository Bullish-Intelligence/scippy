//! Load one file: read, parse, extract, and add its graph to the engine.

use super::Doc;
use super::finish::lose;
use crate::bindings::BindingEngine;
use crate::extract::Extractor;
use crate::language::{Analysis, ProjectModel};
use crate::project::FoundFile;
use crate::report::{Diagnostic, IndexReport, Severity, Stage};
use crate::source::SourceStore;
use std::sync::Arc;

/// Read the text of a file from the store. When the read fails, count the file as skipped, lose
/// it, add a diagnostic, and return `None`.
pub(super) fn read_source(
    store: &SourceStore,
    file: &FoundFile,
    report: &mut IndexReport,
) -> Option<Arc<str>> {
    match store.get(file) {
        Ok(stored) => Some(Arc::clone(&stored.text)),
        Err((code, message)) => {
            report.files_skipped += 1;
            lose(report, &file.path, code);
            report.diagnostics.push(Diagnostic {
                severity: Severity::Warning,
                stage: Stage::Discover,
                file: Some(file.path.as_str().to_string()),
                range: None,
                code,
                message: message.clone(),
            });
            None
        }
    }
}

/// Read, parse, and extract one file, and add its graph to the engine.
/// Return `None` when the pipeline skips the file. A diagnostic then says why.
#[allow(clippy::too_many_arguments)]
pub(super) fn load_file(
    analysis: &dyn Analysis,
    extractors: &[Extractor],
    model: &dyn ProjectModel,
    engine: &mut BindingEngine,
    store: &SourceStore,
    file: &FoundFile,
    report: &mut IndexReport,
    resolve: bool,
) -> Option<Doc> {
    let diagnostic = |stage, severity, code, range, message: String| Diagnostic {
        severity,
        stage,
        file: Some(file.path.as_str().to_string()),
        range,
        code,
        message,
    };
    if let Some(winner) = model.shadowed_by(&file.path) {
        report.files_shadowed += 1;
        lose(report, &file.path, "shadowed-file");
        report.diagnostics.push(diagnostic(
            Stage::Identity,
            Severity::Warning,
            "shadowed-file",
            None,
            format!(
                "skipped: {} shadows this file, so no import reaches it",
                winner.as_str()
            ),
        ));
        return None;
    }
    let source = read_source(store, file, report)?;
    let dialect = analysis.dialect_of(&file.path).min(extractors.len() - 1);
    let extractor = &extractors[dialect];
    // The text that the parser and the resolver read. It differs from `source` only when
    // the pack repaired a construct that its grammar rejects.
    let mut parsed_text: Option<String> = None;
    let first = extractor.parse(&source);
    let tree = match first {
        Ok(tree) => tree,
        Err(error) => {
            let repaired = analysis.repair_syntax(&source).filter(|fixed| {
                fixed.len() == source.len()
                    && fixed.bytes().filter(|&b| b == b'\n').count()
                        == source.bytes().filter(|&b| b == b'\n').count()
            });
            let tree = repaired.and_then(|fixed| {
                let tree = extractor.parse(&fixed).ok()?;
                parsed_text = Some(fixed);
                Some(tree)
            });
            if let Some(tree) = tree {
                report.diagnostics.push(diagnostic(
                    Stage::Parse,
                    Severity::Warning,
                    "repaired-syntax",
                    Some((error.start_byte, error.end_byte)),
                    "the grammar rejects this file. The pack repaired the syntax that it does not know. A repair can blank text, and a name in blanked text gets no occurrence"
                        .to_string(),
                ));
                tree
            } else {
                report.files_skipped += 1;
                lose(report, &file.path, "syntax-error");
                report.diagnostics.push(diagnostic(
                    Stage::Parse,
                    Severity::Warning,
                    "syntax-error",
                    Some((error.start_byte, error.end_byte)),
                    "skipped: the file has a syntax error".to_string(),
                ));
                return None;
            }
        }
    };
    let text = parsed_text.as_deref().unwrap_or(&source);
    let facts = extractor.facts(&file.path, text, &tree);
    drop(tree);
    if let Err(failure) = engine.add_file(
        dialect,
        &file.path,
        text,
        &model.resolver_globals(&file.path),
        resolve,
    ) {
        report.files_skipped += 1;
        lose(report, &file.path, "bind-error");
        report.diagnostics.push(diagnostic(
            Stage::Bind,
            Severity::Warning,
            "bind-error",
            None,
            format!("skipped: {}", failure.message),
        ));
        return None;
    }
    for &(start, end) in &facts.conflicts {
        report.diagnostics.push(diagnostic(
            Stage::Extract,
            Severity::Warning,
            "declaration-conflict",
            Some((start, end)),
            "two captures give this identifier different kinds. The extractor dropped it"
                .to_string(),
        ));
    }
    Some(Doc {
        path: file.path.clone(),
        source,
        facts,
        declared: Vec::new(),
    })
}
