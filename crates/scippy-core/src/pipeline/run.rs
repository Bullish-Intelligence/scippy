//! The orchestration of one run: discover, index each pack, merge, and write.

use super::finish::lose;
use super::inputs::read_support_inputs;
use super::native::index_native_pack;
use super::stack::index_pack;
use super::{PackRoute, PackRun};
use crate::IndexRequest;
use crate::cache::CacheGate;
use crate::emit::{self, Document, IndexData, SymbolInfo};
use crate::error::ScippyError;
use crate::identity::SymbolId;
use crate::language::{Engine, LanguagePack};
use crate::project::{self, FoundFile, Router};
use crate::report::{Diagnostic, IndexReport, Severity, Stage};
use crate::snapshot::{AnalysisSnapshot, Capture, RunContext};
use crate::source::SourceStore;
use std::collections::{BTreeMap, HashMap};
use std::fs::File;
use std::io::Write;
use std::sync::Arc;

pub(crate) fn run(
    request: &IndexRequest,
    languages: &[&dyn LanguagePack],
    capture: bool,
    write_output: bool,
) -> Result<(IndexReport, Option<AnalysisSnapshot>), ScippyError> {
    // A pack that is not enabled for this project takes no part in the run.
    let languages: Vec<&dyn LanguagePack> = languages
        .iter()
        .copied()
        .filter(|p| p.enabled(&request.root))
        .collect();
    let languages = &languages[..];
    let routes: Vec<PackRoute> = languages
        .iter()
        .map(|p| PackRoute {
            pack: *p,
            rules: p.walk_rules(&request.root),
        })
        .collect();
    let routers: Vec<&dyn Router> = routes.iter().map(|r| r as &dyn Router).collect();
    let mut extra_files = Vec::new();
    let mut extra_diagnostics = Vec::new();
    let mut extra_states: Vec<Option<Box<dyn std::any::Any + Send + Sync>>> = Vec::new();
    let mut shards = Vec::new();
    for pack in languages {
        let extra = pack.extra_files(&request.root);
        extra_files.extend(extra.files);
        extra_diagnostics.extend(extra.diagnostics);
        shards.extend(extra.shards);
        extra_states.push(extra.state);
    }
    let found = project::discover(
        &request.root,
        &request.output,
        &routers,
        request.limits,
        &extra_files,
    )?;

    // Hash the support files now, before any stage reads them. A file that changes during the
    // run then differs from its stored hash, and `check` reports it.
    let support_inputs = read_support_inputs(&found.root, languages, &shards, &extra_files);

    let store = SourceStore::new(
        &found.root,
        &found.files,
        request.limits.max_file_bytes,
        request.read_hook.clone(),
    );
    let cache = Arc::new(CacheGate::new(
        request.graph_cache.as_deref(),
        &request.root,
        request.graph_cache_timeout,
        request.graph_cache_max_bytes,
    ));
    let mut report = IndexReport {
        files_discovered: found.files_discovered,
        discovery_skipped: found.files_skipped,
        diagnostics: found.diagnostics,
        lost_files: found.lost,
        ..Default::default()
    };
    report.diagnostics.extend(extra_diagnostics);
    let mut documents: Vec<Document> = Vec::new();
    let mut stubs: Vec<Document> = Vec::new();
    let mut external_symbols: BTreeMap<SymbolId, SymbolInfo> = BTreeMap::new();
    let mut trace = Vec::new();
    // A strict run needs the snapshot, because its flag `input_complete` decides the result.
    let mut capture = (capture || request.strict).then(Capture::default);

    for (index, pack) in languages.iter().enumerate() {
        let files: Vec<&FoundFile> = found.files.iter().filter(|f| f.pack == index).collect();
        if files.is_empty() {
            continue;
        }
        let Some(engine) = pack.engine() else {
            report.files_skipped += files.len();
            for file in &files {
                lose(&mut report, &file.path, "unsupported-language");
            }
            report.diagnostics.push(Diagnostic {
                severity: Severity::Warning,
                stage: Stage::Discover,
                file: None,
                range: None,
                code: "unsupported-language",
                message: format!(
                    "the {} pack does not support indexing yet: skipped {} file(s)",
                    pack.id(),
                    files.len()
                ),
            });
            continue;
        };
        let mut run = PackRun {
            pack: *pack,
            root: &found.root,
            files: &files,
            store: &store,
            request,
            report: &mut report,
            trace: &mut trace,
            capture: &mut capture,
        };
        let indexed = match engine {
            Engine::StackGraphs(analysis) => {
                index_pack(&mut run, analysis, &shards, &extra_files, &cache)?
            }
            Engine::Native(native) => index_native_pack(
                &mut run,
                native,
                &found.all_files,
                extra_states[index].as_deref(),
            )?,
        };
        documents.extend(indexed.documents);
        stubs.extend(indexed.stubs);
        external_symbols.extend(indexed.external_symbols);
    }
    report.graph_cache_failures = cache.failures();
    // A stub defines a symbol in a file that another pack may own. Merge it into that document.
    for stub in stubs {
        match documents
            .iter_mut()
            .find(|d: &&mut Document| d.path == stub.path)
        {
            Some(own) => {
                own.occurrences.extend(stub.occurrences);
                own.symbols.extend(stub.symbols);
            }
            None => documents.push(stub),
        }
    }

    // The pack and the real path of each routed file.
    let routed: HashMap<String, (String, Option<String>)> = found
        .files
        .iter()
        .map(|f| {
            (
                f.path.as_str().to_owned(),
                (
                    languages[f.pack].id().to_owned(),
                    f.real.as_ref().map(|p| p.to_string_lossy().into_owned()),
                ),
            )
        })
        .collect();

    // Check each indexed file against the disk once more. The run indexed the content that it
    // read first, so a file that changed since is stale, not lost. The facts stay.
    report.stale_files = store.stale(
        documents
            .iter()
            .filter(|d| !d.language.is_empty())
            .map(|d| d.path.as_str()),
    );
    for path in &report.stale_files {
        report.diagnostics.push(Diagnostic {
            severity: Severity::Warning,
            stage: Stage::Discover,
            file: Some(path.clone()),
            range: None,
            code: "stale-file",
            message: "the file on disk differs from the content that the run indexed".to_string(),
        });
    }
    // Complete the list of the lost files: one entry for each path, in the order of the paths.
    for lost in &mut report.lost_files {
        if let Some((pack, real)) = routed.get(&lost.path) {
            lost.pack.get_or_insert_with(|| pack.clone());
            if lost.real_path.is_none() {
                lost.real_path = real.clone();
            }
        }
    }
    report.lost_files.sort_by(|a, b| a.path.cmp(&b.path));
    report.lost_files.dedup_by(|b, a| a.path == b.path);

    report.files_indexed = documents.len();
    report.occurrences = documents.iter().map(|d| d.occurrences.len()).sum();

    let context = RunContext {
        project_root: found.root.to_string_lossy().into_owned(),
        shards: {
            let mut names: Vec<String> = shards.iter().map(|s| s.as_str().to_owned()).collect();
            names.sort();
            names.dedup();
            names
        },
        options: format!(
            "input_limits={:?};resolve_limits={:?};package_name={:?};package_version={:?}",
            request.limits, request.resolve_limits, request.package_name, request.package_version
        ),
        support_inputs,
        files: routed,
    };
    let data = IndexData {
        project_root: found.root,
        documents,
        external_symbols: external_symbols.into_values().collect(),
    };
    let snapshot = if let Some(capture) = capture {
        Some(AnalysisSnapshot::new(&data, capture, &report, context)?)
    } else {
        None
    };
    // A strict run fails when it lost input, and it writes no file.
    if request.strict && snapshot.as_ref().is_some_and(|s| !s.input_complete) {
        return Err(ScippyError::Incomplete);
    }
    if let Some(path) = &request.trace {
        trace.sort();
        let io = |source| ScippyError::Io {
            path: path.clone(),
            source,
        };
        let mut file = File::create(path).map_err(io)?;
        for line in trace {
            writeln!(file, "{line}").map_err(io)?;
        }
    }

    if write_output {
        emit::write_index(data, &request.output)?;
    }
    Ok((report, snapshot))
}
