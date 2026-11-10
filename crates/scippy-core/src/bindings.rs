//! The binding engine adapter. It is the only module that knows Stack Graphs.
//!
//! The adapter builds one stack graph for each file, stores the partial paths of every file
//! in one database, and resolves each reference node with bounded work.
//! It returns plain anchors. No fork type leaves this module.

use std::collections::{BTreeSet, HashMap, HashSet};
use std::panic::{AssertUnwindSafe, catch_unwind};
use std::path::PathBuf;
use std::rc::Rc;

use stack_graphs::arena::Handle;
use stack_graphs::graph::{File, Node, SourceInfo, StackGraph};
use stack_graphs::partial::{PartialPath, PartialPaths};
use stack_graphs::stitching::{
    Database, ForwardPartialPathStitcher, StitchBound, StitcherConfig, StitcherLimits,
};
use tree_sitter_stack_graphs::{BuildError, NoCancellation, StackGraphLanguage, Variables};

use crate::error::ScippyError;
use crate::extract::SourceAnchor;
use crate::language::{Analysis, BindingRules, SupportFile};
use crate::project::ProjectRelativePath;

/// The name that the rules use for the builtins file.
pub(crate) const BUILTINS_NAME: &str = "<builtins>";

/// Limits for the work on one reference.
///
/// The Python rules of the fork at 4738172 add paths for annotations and base classes.
/// The old limits (30,000 work, 5,000,000 arena cells) stopped about 1,500 more mypy
/// references than before. These limits bring the count back to the old level.
#[derive(Clone, Copy, Debug)]
pub(crate) struct ResolveLimits {
    /// Units of work: one for each path that the search extends, one for each candidate.
    pub max_work: usize,
    /// Paths that wait for extension.
    pub max_queue_len: usize,
    /// Cells that one search adds to the path arenas. This bounds the memory of the search.
    pub max_arena_cells: usize,
}

impl Default for ResolveLimits {
    fn default() -> Self {
        Self {
            max_work: 100_000,
            max_queue_len: 2_000,
            max_arena_cells: 20_000_000,
        }
    }
}

/// Why a search stopped before it finished. A bounded result can lack ends.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Bound {
    Work,
    QueueLength,
    ArenaCells,
    /// The search panicked. The adapter caught the panic.
    Failed,
}

/// The site of a definition node: a span in a project file or in an external asset.
///
/// An external asset is a source file that the pack supplies and the project does not own.
/// The builtins file is an example.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub(crate) struct Place {
    pub anchor: SourceAnchor,
    /// True when the span belongs to an external asset.
    pub external: bool,
}

/// The end of a binding path.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub(crate) struct RawEnd {
    /// The definition node. `None` when the node has no source span.
    pub place: Option<Place>,
    /// The kind that the rules gave to the definition. It serves diagnostics only.
    pub syntax_type: Option<String>,
    /// The definition nodes that the path passes before it reaches the end, sorted.
    /// A path that passes a definition follows a value from that definition to the end.
    pub through: Vec<Place>,
}

/// What the engine found for one reference node.
#[derive(Clone, Debug)]
pub(crate) struct RawBinding {
    pub reference: SourceAnchor,
    /// The distinct ends of the paths that no other path shadows, sorted.
    pub ends: Vec<RawEnd>,
    /// The distinct ends that only shadowed paths reach, sorted. The engine rules give the
    /// paths a precedence. A path with a higher precedence shadows a path with a lower one,
    /// for example a local definition that hides a definition of an outer scope.
    pub shadowed_ends: Vec<RawEnd>,
    pub bound: Option<Bound>,
}

/// Why the engine did not build a file.
#[derive(Debug)]
pub(crate) struct BuildFailure {
    pub message: String,
}

/// The compiled binding rules of a pack: one rule set for each dialect.
///
/// Compiling the rules is costly. The pipeline compiles them once and shares them with the
/// engine of each shard.
#[derive(Clone)]
pub(crate) struct CompiledRules {
    /// The first rule set is the primary dialect.
    languages: Rc<Vec<StackGraphLanguage>>,
}

impl CompiledRules {
    /// Compile the rules of a pack.
    pub fn new(pack: &'static str, analysis: &dyn Analysis) -> Result<Self, ScippyError> {
        let rules = analysis.binding_rules();
        let bad = |reason: String| ScippyError::Pack { pack, reason };
        let compile = |grammar, path: &str, source| {
            StackGraphLanguage::from_source(grammar, PathBuf::from(path), source)
                .map(|mut language| {
                    // Scippy never reads the debug information of the nodes.
                    language.set_debug_attributes(false);
                    language
                })
                .map_err(|e| bad(format!("the binding rules do not compile: {e}")))
        };
        let mut languages = vec![compile(
            analysis.grammar(),
            rules.tsg_path,
            rules.tsg_source,
        )?];
        for dialect in analysis.extra_dialects() {
            languages.push(compile(
                dialect.grammar,
                dialect.tsg_path,
                dialect.tsg_source,
            )?);
        }
        Ok(Self {
            languages: Rc::new(languages),
        })
    }
}

pub(crate) struct BindingEngine {
    /// The compiled rules. The shards share them.
    languages: Rc<Vec<StackGraphLanguage>>,
    graph: StackGraph,
    partials: PartialPaths,
    database: Database,
    files: Vec<(Handle<File>, ProjectRelativePath)>,
    /// The names of the external assets, such as the builtins. They are not project files.
    external: HashSet<String>,
    /// The pack asks for shadow aware similar path detection in every stitcher.
    shadow_aware: bool,
}

impl BindingEngine {
    /// Start an engine with the compiled rules of a pack and load its builtins.
    pub fn new(
        pack: &'static str,
        analysis: &dyn Analysis,
        compiled: &CompiledRules,
    ) -> Result<Self, ScippyError> {
        let rules = analysis.binding_rules();
        let bad = |reason: String| ScippyError::Pack { pack, reason };
        let mut engine = Self {
            languages: Rc::clone(&compiled.languages),
            graph: StackGraph::new(),
            partials: PartialPaths::new(),
            database: Database::new(),
            files: Vec::new(),
            external: HashSet::new(),
            shadow_aware: rules.shadow_aware_similar_paths,
        };
        engine.load_assets(&rules).map_err(bad)?;
        Ok(engine)
    }

    /// Build each external asset as a file of its own and store its partial paths.
    fn load_assets(&mut self, rules: &BindingRules) -> Result<(), String> {
        for asset in rules.external_assets() {
            if asset.source.trim().is_empty() {
                continue;
            }
            let file = self.graph.get_or_create_file(&asset.name);
            let mut globals = Variables::new();
            globals
                .add("FILE_PATH".into(), asset.name.as_str().into())
                .map_err(|e| e.to_string())?;
            for (name, value) in &asset.globals {
                globals
                    .add(name.as_str().into(), value.as_str().into())
                    .map_err(|e| e.to_string())?;
            }
            self.languages[0]
                .build_stack_graph_into(
                    &mut self.graph,
                    file,
                    asset.source,
                    &globals,
                    &NoCancellation,
                )
                .map_err(|e| format!("the asset source {} fails to build: {e}", asset.path))?;
            self.add_partial_paths(file);
            self.external.insert(asset.name);
        }
        Ok(())
    }

    /// Build the graph of one file and store its partial paths.
    ///
    /// The engine resolves the references of the file only when `resolve` is true. A file of a
    /// dependency only gives definitions. On failure the engine never resolves a reference of
    /// the file.
    pub fn add_file(
        &mut self,
        dialect: usize,
        path: &ProjectRelativePath,
        source: &str,
        globals: &[(String, String)],
        resolve: bool,
    ) -> Result<(), BuildFailure> {
        let file = self.graph.get_or_create_file(path.as_str());
        let mut variables = Variables::new();
        let fail = |message: String| BuildFailure { message };
        variables
            .add("FILE_PATH".into(), path.as_str().into())
            .map_err(|e| fail(e.to_string()))?;
        for (name, value) in globals {
            variables
                .add(name.as_str().into(), value.as_str().into())
                .map_err(|e| fail(e.to_string()))?;
        }
        let language = &self.languages[dialect];
        let built = catch_unwind(AssertUnwindSafe(|| {
            language.build_stack_graph_into(
                &mut self.graph,
                file,
                source,
                &variables,
                &NoCancellation,
            )
        }));
        match built {
            Ok(Ok(())) => {}
            Ok(Err(BuildError::ParseErrors(_) | BuildError::ParseError)) => {
                return Err(fail("the source has syntax errors".into()));
            }
            Ok(Err(error)) => return Err(fail(error.to_string())),
            Err(_) => return Err(fail("the rules panicked".into())),
        }
        self.add_partial_paths(file);
        if resolve {
            self.files.push((file, path.clone()));
        }
        Ok(())
    }

    /// Add the nodes of a support file, such as `tsconfig.json`, and store its partial paths.
    ///
    /// `all_paths` lists the source files of the project.
    pub fn add_support_file(
        &mut self,
        support: &SupportFile,
        all_paths: &[ProjectRelativePath],
    ) -> Result<(), BuildFailure> {
        let fail = |message: String| BuildFailure { message };
        let file = self.graph.get_or_create_file(support.path.as_str());
        let path = PathBuf::from(support.path.as_str());
        let paths: Vec<PathBuf> = all_paths
            .iter()
            .map(|p| PathBuf::from(p.as_str()))
            .collect();
        let globals = HashMap::new();
        let built = catch_unwind(AssertUnwindSafe(|| {
            support.analyzer.build_stack_graph_into(
                &mut self.graph,
                file,
                &path,
                &support.source,
                &mut paths.iter().map(PathBuf::as_path),
                &globals,
                &NoCancellation,
            )
        }));
        match built {
            Ok(Ok(())) => {}
            Ok(Err(error)) => return Err(fail(error.to_string())),
            Err(_) => return Err(fail("the analyzer panicked".into())),
        }
        self.add_partial_paths(file);
        Ok(())
    }

    fn add_partial_paths(&mut self, file: Handle<File>) {
        let mut paths: Vec<PartialPath> = Vec::new();
        let result = ForwardPartialPathStitcher::find_minimal_partial_path_set_in_file(
            &self.graph,
            &mut self.partials,
            file,
            StitcherConfig::default().with_shadow_aware_similar_paths(self.shadow_aware),
            &stack_graphs::NoCancellation,
            |_, _, path| paths.push(path.clone()),
        );
        if result.is_ok() {
            for path in paths {
                self.database
                    .add_partial_path(&self.graph, &mut self.partials, path);
            }
        }
    }

    /// Resolve every reference node of every file that built.
    ///
    /// The result lists the files in the order of `add_file`, and the nodes of a file in graph order.
    pub fn resolve(&mut self, limits: ResolveLimits) -> Vec<RawBinding> {
        let mut out = Vec::new();
        let config = StitcherConfig::default()
            .with_shadow_aware_similar_paths(self.shadow_aware)
            .with_limits(StitcherLimits {
                max_work: Some(limits.max_work),
                max_queue_len: Some(limits.max_queue_len),
                max_arena_cells: Some(limits.max_arena_cells),
                max_path_length: None,
            });
        let files = std::mem::take(&mut self.files);
        let external = &self.external;
        for (file, path) in &files {
            let references: Vec<Handle<Node>> = self
                .graph
                .nodes_for_file(*file)
                .filter(|&n| self.graph[n].is_reference())
                .collect();
            for node in references {
                let Some(reference) = self.anchor(node, path) else {
                    continue;
                };
                let mut kept: Vec<(PartialPath, RawEnd)> = Vec::new();
                let mut all: BTreeSet<RawEnd> = BTreeSet::new();
                let searched = catch_unwind(AssertUnwindSafe(|| {
                    ForwardPartialPathStitcher::find_complete_partial_paths_in_scope(
                        &self.graph,
                        &mut self.partials,
                        &self.database,
                        node,
                        config,
                        &stack_graphs::NoCancellation,
                        |graph, partials, path: &PartialPath| {
                            // Drop every path that another path shadows. The engine does not
                            // do it. Its own test runner applies the same filter.
                            let end = end_of(graph, partials, path, external);
                            all.insert(end.clone());
                            if kept.iter().any(|(other, _)| other.shadows(partials, path)) {
                                return;
                            }
                            kept.retain(|(other, _)| !path.shadows(partials, other));
                            kept.push((path.clone(), end));
                        },
                    )
                }));
                let bound = match searched {
                    Ok(Ok(stats)) => stats.bounded.map(|b| match b {
                        StitchBound::Work => Bound::Work,
                        StitchBound::QueueLength => Bound::QueueLength,
                        StitchBound::ArenaCells => Bound::ArenaCells,
                        // The search dropped a long path and went on. It has no path-length
                        // limit here, so this value does not occur. Treat it as a failure.
                        StitchBound::PathLength => Bound::Failed,
                    }),
                    Ok(Err(_)) | Err(_) => Some(Bound::Failed),
                };
                let ends: BTreeSet<RawEnd> = kept.into_iter().map(|(_, end)| end).collect();
                let shadowed_ends = all.difference(&ends).cloned().collect();
                out.push(RawBinding {
                    reference,
                    ends: ends.into_iter().collect(),
                    shadowed_ends,
                    bound,
                });
            }
        }
        self.files = files;
        out
    }

    fn anchor(&self, node: Handle<Node>, path: &ProjectRelativePath) -> Option<SourceAnchor> {
        let info = self.graph.source_info(node)?;
        anchor_from_info(info, path)
    }
}

fn anchor_from_info(info: &SourceInfo, path: &ProjectRelativePath) -> Option<SourceAnchor> {
    let span = &info.span;
    let start = span.start.containing_line.start + span.start.column.utf8_offset;
    let end = span.end.containing_line.start + span.end.column.utf8_offset;
    Some(SourceAnchor {
        path: path.clone(),
        start_byte: u32::try_from(start).ok()?,
        end_byte: u32::try_from(end).ok()?,
    })
}

fn place_of(
    graph: &StackGraph,
    node: Handle<Node>,
    external_files: &HashSet<String>,
) -> Option<Place> {
    let file = graph[node].file()?;
    let info = graph.source_info(node)?;
    let name = graph[file].name();
    let external = external_files.contains(name);
    let path = ProjectRelativePath::new(name).ok()?;
    let anchor = anchor_from_info(info, &path)?;
    Some(Place { anchor, external })
}

fn end_of(
    graph: &StackGraph,
    partials: &PartialPaths,
    path: &PartialPath,
    external: &HashSet<String>,
) -> RawEnd {
    let end = path.end_node;
    let place = place_of(graph, end, external);
    // The edges of the path name every node of the path except the last one.
    let mut through: Vec<Place> = path
        .edges
        .iter_unordered(partials)
        .filter_map(|edge| graph.node_for_id(edge.source_node_id))
        .filter(|&node| graph[node].is_definition())
        .filter_map(|node| place_of(graph, node, external))
        .filter(|p| Some(p) != place.as_ref())
        .collect();
    through.sort();
    through.dedup();
    RawEnd {
        place,
        syntax_type: graph
            .source_info(end)
            .and_then(|info| info.syntax_type.into_option())
            .map(|h| graph[h].to_string()),
        through,
    }
}
