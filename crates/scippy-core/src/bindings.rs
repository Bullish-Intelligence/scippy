//! The binding engine adapter. It is the only module that knows Stack Graphs.
//!
//! The adapter builds one stack graph for each file, stores the partial paths of every file
//! in one database, and resolves each reference node with bounded work.
//! It returns plain anchors. No fork type leaves this module.

use std::cell::RefCell;
use std::collections::{BTreeSet, HashMap, HashSet};
use std::panic::{AssertUnwindSafe, catch_unwind};
use std::path::PathBuf;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, OnceLock};

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use stack_graphs::arena::Handle;
use stack_graphs::graph::{File, Node, SourceInfo, StackGraph, Symbol};
use stack_graphs::partial::{FrozenPartialPaths, PartialPath, PartialPaths};
use stack_graphs::stitching::{
    Database, ForwardPartialPathStitcher, StitchBound, StitcherConfig, StitcherLimits,
};
use tree_sitter_stack_graphs::{BuildError, NoCancellation, StackGraphLanguage, Variables};

use crate::cache::{CacheGate, CachedGraph, GraphCache, decode, encode};
use crate::error::ScippyError;
use crate::extract::SourceAnchor;
use crate::language::{Analysis, BindingRules, SupportFile};
use crate::project::ProjectRelativePath;
use crate::report::SearchWork;

/// The name that the rules use for the builtins file.
pub(crate) const BUILTINS_NAME: &str = "<builtins>";

/// Limits for the work on one reference.
///
/// The Python rules of the fork at 4738172 add paths for annotations and base classes.
/// The old limits (30,000 work, 5,000,000 arena cells) stopped about 1,500 more mypy
/// references than before. The limits of 100,000 work and 20,000,000 arena cells brought
/// the count back to the old level.
/// A later measurement (release build, one run each, no cache) found 81 bounded references
/// in clinch (45 files) and 56 in a 426-file project set (mr). All of them hit the work
/// limit. No search hit the queue limit or the arena limit, so those two stay as they were.
/// A work limit of 300,000 removed all 81 in clinch and 50 of 56 in mr.
/// A work limit of 1,000,000 removed all of them. It raised the peak memory by 8% in clinch
/// and by 4% in mr, and the wall time did not rise beyond the noise of the runs.
/// The arena limit is for one search, and `--jobs` searches run at once.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub struct ResolveLimits {
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
            max_work: 1_000_000,
            max_queue_len: 2_000,
            max_arena_cells: 20_000_000,
        }
    }
}

/// Why a search stopped before it finished. A bounded result can lack ends.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
#[non_exhaustive]
pub enum Bound {
    Work,
    QueueLength,
    ArenaCells,
    /// The search panicked. The adapter caught the panic.
    Failed,
}

impl Bound {
    /// The stable name of the bound, for the trace, the snapshot, and the database.
    pub fn code(self) -> &'static str {
        match self {
            Bound::Work => "work",
            Bound::QueueLength => "queue-length",
            Bound::ArenaCells => "arena-cells",
            Bound::Failed => "failed",
        }
    }

    /// The bound with this stable name, or `None` for an unknown name.
    pub fn from_code(code: &str) -> Option<Self> {
        match code {
            "work" => Some(Bound::Work),
            "queue-length" => Some(Bound::QueueLength),
            "arena-cells" => Some(Bound::ArenaCells),
            "failed" => Some(Bound::Failed),
            _ => None,
        }
    }
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
    /// The definition nodes among `through` whose name is the name of the reference, sorted.
    /// A path that pops the name of the reference at such a node, and goes on, follows the
    /// value that a store gave to that name.
    pub echo: Vec<Place>,
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

/// The binding rules of one dialect. The engine compiles them on first use.
struct DialectRules {
    grammar: tree_sitter::Language,
    path: &'static str,
    source: &'static str,
    compiled: OnceLock<Result<StackGraphLanguage, String>>,
}

impl DialectRules {
    fn new(grammar: tree_sitter::Language, path: &'static str, source: &'static str) -> Self {
        Self {
            grammar,
            path,
            source,
            compiled: OnceLock::new(),
        }
    }

    /// Compile the rules once and return the result.
    fn language(&self) -> Result<&StackGraphLanguage, &str> {
        self.compiled
            .get_or_init(|| {
                StackGraphLanguage::from_source(
                    self.grammar.clone(),
                    PathBuf::from(self.path),
                    self.source,
                )
                .map(|mut language| {
                    // Scippy never reads the debug information of the nodes.
                    language.set_debug_attributes(false);
                    language
                })
                .map_err(|e| format!("the binding rules do not compile: {e}"))
            })
            .as_ref()
            .map_err(String::as_str)
    }
}

/// The compiled binding rules of a pack: one rule set for each dialect.
///
/// Compiling the rules is costly: it takes about a second for each dialect. Three steps cut
/// the cost.
/// - The pipeline compiles the rules once for each worker and shares them with the engine of
///   each shard.
/// - A dialect compiles when the first file of that dialect needs it. A project with no TSX
///   file never compiles the TSX rules.
/// - A thread keeps the rules that it compiled. A second run on the same thread, for example
///   a second index call of a test, compiles nothing.
#[derive(Clone)]
pub(crate) struct CompiledRules {
    /// The first rule set is the primary dialect.
    dialects: Arc<Vec<DialectRules>>,
}

thread_local! {
    /// The rules that this thread compiled. The key names the sources of the rules.
    static COMPILED: RefCell<HashMap<Vec<(usize, usize)>, CompiledRules>> =
        RefCell::new(HashMap::new());
}

impl CompiledRules {
    /// Get the rules of a pack. Compile the primary dialect now, so that a pack with bad
    /// rules fails at the start of the run.
    pub fn new(pack: &'static str, analysis: &dyn Analysis) -> Result<Self, ScippyError> {
        let rules = analysis.binding_rules();
        let extra = analysis.extra_dialects();
        // The sources are static text, so their addresses tell the rule sets apart.
        let key: Vec<(usize, usize)> = std::iter::once(rules.tsg_source)
            .chain(extra.iter().map(|d| d.tsg_source))
            .map(|source| (source.as_ptr() as usize, source.len()))
            .collect();
        if let Some(found) = COMPILED.with(|cache| cache.borrow().get(&key).cloned()) {
            return Ok(found);
        }
        let mut dialects = vec![DialectRules::new(
            analysis.grammar(),
            rules.tsg_path,
            rules.tsg_source,
        )];
        for dialect in extra {
            dialects.push(DialectRules::new(
                dialect.grammar,
                dialect.tsg_path,
                dialect.tsg_source,
            ));
        }
        if let Err(reason) = dialects[0].language() {
            return Err(ScippyError::Pack {
                pack,
                reason: reason.to_owned(),
            });
        }
        let compiled = Self {
            dialects: Arc::new(dialects),
        };
        COMPILED.with(|cache| cache.borrow_mut().insert(key, compiled.clone()));
        Ok(compiled)
    }
}

/// The key of the inputs that decide the graphs of the external assets.
///
/// A blob from [`prebuild_assets`] starts with this key. The key covers the rules and each
/// asset, so a blob from other inputs never loads.
fn assets_key(rules: &BindingRules) -> String {
    let assets: Vec<_> = rules
        .external_assets()
        .into_iter()
        .map(|a| (a.name, a.source, a.globals))
        .collect();
    let input = serde_json::to_vec(&(
        "asset-graphs-v1",
        rules.tsg_source,
        assets,
        rules.shadow_aware_similar_paths,
    ))
    .expect("the key input encodes");
    format!("{:x}", Sha256::digest(input))
}

/// Build the graphs of the external assets of a pack, such as its builtins, and return them as
/// bytes. A build script calls this. The pack then sets
/// [`BindingRules::prebuilt_assets`](crate::BindingRules::prebuilt_assets) to the bytes, and each
/// run loads the graphs instead of building them.
///
/// `grammar` and `rules` are the primary dialect and the rules of the pack.
pub fn prebuild_assets(
    grammar: tree_sitter::Language,
    rules: &BindingRules,
) -> Result<Vec<u8>, String> {
    let dialects = vec![DialectRules::new(grammar, rules.tsg_path, rules.tsg_source)];
    dialects[0].language().map_err(str::to_owned)?;
    let compiled = CompiledRules {
        dialects: Arc::new(dialects),
    };
    let gate = Arc::new(CacheGate::new(
        None,
        std::path::Path::new("."),
        std::time::Duration::ZERO,
        0,
    ));
    let (mut engine, paths) =
        BindingEngine::start("prebuild", None, rules, &compiled, &gate, false)
            .map_err(|e| e.to_string())?;
    let graph = stack_graphs::serde::StackGraph::from_graph(&engine.graph);
    let paths = paths
        .iter()
        .map(|p| {
            stack_graphs::serde::PartialPath::from_partial_path(
                &engine.graph,
                &mut engine.partials,
                p,
            )
        })
        .collect();
    let mut blob = assets_key(rules).into_bytes();
    blob.extend(encode(&CachedGraph { graph, paths })?);
    Ok(blob)
}

/// The graph of one file and its minimal partial paths, built apart from the engine.
pub(crate) struct BuiltFile {
    graph: CachedGraph,
    /// False when the search for the minimal partial paths failed. The graph then has no paths,
    /// as in a serial build, and the graph cache never stores it.
    cacheable: bool,
}

/// The state that a cache lookup gives for one file.
pub(crate) enum Lookup {
    /// The cache has the graph of the file.
    Hit(CachedGraph),
    /// The file needs a build. `key` holds the cache key and owner when the build can be stored.
    Miss { key: Option<(String, String)> },
}

/// Builds files without a binding engine. The rules are shared, so the builder is `Sync`: all
/// worker threads of a shard use one builder. A build never touches another file, so the files
/// of a shard can build at once.
pub(crate) struct FileBuilder {
    languages: Arc<Vec<DialectRules>>,
    shadow_aware: bool,
}

impl FileBuilder {
    pub fn new(analysis: &dyn Analysis, compiled: &CompiledRules) -> Self {
        Self {
            languages: Arc::clone(&compiled.dialects),
            shadow_aware: analysis.binding_rules().shadow_aware_similar_paths,
        }
    }

    /// Build the graph of one file in a fresh stack graph and find its minimal partial paths.
    /// The result matches the entry that the graph cache stores for the file. A failure has the
    /// message that `BindingEngine::add_file` gives.
    pub fn build(
        &self,
        dialect: usize,
        path: &ProjectRelativePath,
        source: &str,
        globals: &[(String, String)],
    ) -> Result<BuiltFile, BuildFailure> {
        let fail = |message: String| BuildFailure { message };
        let mut graph = StackGraph::new();
        let mut partials = PartialPaths::new();
        let file = graph.get_or_create_file(path.as_str());
        let mut variables = Variables::new();
        variables
            .add("FILE_PATH".into(), path.as_str().into())
            .map_err(|e| fail(e.to_string()))?;
        for (name, value) in globals {
            variables
                .add(name.as_str().into(), value.as_str().into())
                .map_err(|e| fail(e.to_string()))?;
        }
        let language = self.languages[dialect]
            .language()
            .map_err(|e| fail(e.to_owned()))?;
        let built = catch_unwind(AssertUnwindSafe(|| {
            language.build_stack_graph_into(&mut graph, file, source, &variables, &NoCancellation)
        }));
        match built {
            Ok(Ok(())) => {}
            Ok(Err(BuildError::ParseErrors(_) | BuildError::ParseError)) => {
                return Err(fail("the source has syntax errors".into()));
            }
            Ok(Err(error)) => return Err(fail(error.to_string())),
            Err(_) => return Err(fail("the rules panicked".into())),
        }
        let mut paths: Vec<PartialPath> = Vec::new();
        let result = ForwardPartialPathStitcher::find_minimal_partial_path_set_in_file(
            &graph,
            &mut partials,
            file,
            StitcherConfig::default().with_shadow_aware_similar_paths(self.shadow_aware),
            &stack_graphs::NoCancellation,
            |_, _, path| paths.push(path.clone()),
        );
        let cacheable = result.is_ok();
        if !cacheable {
            paths.clear();
        }
        let serial_graph =
            stack_graphs::serde::StackGraph::from_graph_filter(&graph, &|_: &StackGraph,
                                                                         candidate: &Handle<
                File,
            >| {
                *candidate == file
            });
        let serial_paths = paths
            .iter()
            .map(|p| stack_graphs::serde::PartialPath::from_partial_path(&graph, &mut partials, p))
            .collect();
        Ok(BuiltFile {
            graph: CachedGraph {
                graph: serial_graph,
                paths: serial_paths,
            },
            cacheable,
        })
    }
}

pub(crate) struct BindingEngine {
    pack: &'static str,
    cache_key: Option<String>,
    /// The compiled rules. The shards share them.
    languages: Arc<Vec<DialectRules>>,
    graph: StackGraph,
    partials: PartialPaths,
    database: Database,
    files: Vec<(Handle<File>, ProjectRelativePath)>,
    /// The names of the external assets, such as the builtins. They are not project files.
    external: HashSet<String>,
    /// The pack asks for shadow aware similar path detection in every stitcher.
    shadow_aware: bool,
    /// The pack asks for a cap on the scope depth in the resolve step.
    max_scope_depth: Option<usize>,
    cache: Option<GraphCache>,
    /// The run-wide cache state. A cache failure turns the cache off for all shards.
    gate: Arc<CacheGate>,
    pub cache_hits: usize,
    pub cache_misses: usize,
}

impl BindingEngine {
    /// Start an engine with the compiled rules of a pack and load its builtins.
    pub fn new(
        pack: &'static str,
        analysis: &dyn Analysis,
        compiled: &CompiledRules,
        gate: &Arc<CacheGate>,
    ) -> Result<Self, ScippyError> {
        let rules = analysis.binding_rules();
        Self::start(
            pack,
            analysis.graph_cache_key(),
            &rules,
            compiled,
            gate,
            true,
        )
        .map(|(engine, _)| engine)
    }

    fn start(
        pack: &'static str,
        cache_key: Option<String>,
        rules: &BindingRules,
        compiled: &CompiledRules,
        gate: &Arc<CacheGate>,
        use_prebuilt: bool,
    ) -> Result<(Self, Vec<PartialPath>), ScippyError> {
        let bad = |reason: String| ScippyError::Pack { pack, reason };
        let cache_enabled = cache_key.is_some();
        let mut engine = Self {
            pack,
            cache_key,
            languages: Arc::clone(&compiled.dialects),
            graph: StackGraph::new(),
            partials: PartialPaths::new(),
            database: Database::new(),
            files: Vec::new(),
            external: HashSet::new(),
            shadow_aware: rules.shadow_aware_similar_paths,
            max_scope_depth: rules.max_scope_depth,
            cache: if cache_enabled { gate.open() } else { None },
            gate: Arc::clone(gate),
            cache_hits: 0,
            cache_misses: 0,
        };
        if use_prebuilt && let Some(blob) = rules.prebuilt_assets {
            // A blob that does not match the rules, or that fails to load, never serves. The
            // engine then starts again with an empty graph and builds the assets itself.
            if engine.load_prebuilt(rules, blob).is_ok() {
                return Ok((engine, Vec::new()));
            }
            engine.graph = StackGraph::new();
            engine.partials = PartialPaths::new();
            engine.database = Database::new();
            engine.external.clear();
        }
        let paths = engine.load_assets(rules).map_err(bad)?;
        Ok((engine, paths))
    }

    /// Load the graphs that [`prebuild_assets`] made, when they match the rules.
    fn load_prebuilt(&mut self, rules: &BindingRules, blob: &[u8]) -> Result<(), String> {
        let key = assets_key(rules);
        let body = blob
            .strip_prefix(key.as_bytes())
            .ok_or("the prebuilt assets do not match the rules")?;
        let cached = decode(body)?;
        self.load_cached(&cached)?;
        for asset in rules.external_assets() {
            if !asset.source.trim().is_empty() {
                self.external.insert(asset.name);
            }
        }
        Ok(())
    }

    /// Add a stored graph and its partial paths to the engine.
    fn load_cached(&mut self, cached: &CachedGraph) -> Result<(), String> {
        cached
            .graph
            .load_into(&mut self.graph)
            .map_err(|e| e.to_string())?;
        for path in &cached.paths {
            let path = path
                .to_partial_path(&mut self.graph, &mut self.partials)
                .map_err(|e| e.to_string())?;
            self.database
                .add_partial_path(&self.graph, &mut self.partials, path);
        }
        Ok(())
    }

    /// Build each external asset as a file of its own and store its partial paths.
    /// Return the partial paths of all assets.
    fn load_assets(&mut self, rules: &BindingRules) -> Result<Vec<PartialPath>, String> {
        let mut all = Vec::new();
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
            let language = self.languages[0].language().map_err(str::to_owned)?;
            language
                .build_stack_graph_into(
                    &mut self.graph,
                    file,
                    asset.source,
                    &globals,
                    &NoCancellation,
                )
                .map_err(|e| format!("the asset source {} fails to build: {e}", asset.path))?;
            if let Some(paths) = self.add_partial_paths(file) {
                all.extend(paths);
            }
            self.external.insert(asset.name);
        }
        Ok(all)
    }

    /// Look up the graph of a file in the graph cache.
    ///
    /// A miss counts in `cache_misses`. A cache failure turns the cache off for the run, and the
    /// answer is a miss with no key, so the file builds and the engine stores nothing.
    pub fn lookup(
        &mut self,
        dialect: usize,
        path: &ProjectRelativePath,
        source: &str,
        globals: &[(String, String)],
    ) -> Lookup {
        // Another shard may have turned the cache off.
        if !self.gate.is_on() {
            self.cache = None;
        }
        let mut cache_key: Option<(String, String)> = None;
        if let Some(cache) = &self.cache {
            match cache.key(
                self.pack,
                self.cache_key
                    .as_deref()
                    .expect("cache requires a pack key"),
                path.as_str(),
                source,
                globals,
                dialect,
                self.shadow_aware,
            ) {
                Ok(keys) => cache_key = Some(keys),
                Err(error) => {
                    self.gate.fail(&error);
                    self.cache = None;
                }
            }
        }
        if let (Some(cache), Some((key, _))) = (&self.cache, &cache_key) {
            match cache.get(key) {
                Ok(Some(cached)) => return Lookup::Hit(cached),
                Ok(None) => self.cache_misses += 1,
                Err(error) => {
                    self.gate.fail(&error);
                    self.cache = None;
                    cache_key = None;
                }
            }
        }
        Lookup::Miss { key: cache_key }
    }

    /// Add a file whose graph came from the cache.
    pub fn add_cached(
        &mut self,
        path: &ProjectRelativePath,
        cached: &CachedGraph,
        resolve: bool,
    ) -> Result<(), BuildFailure> {
        let loaded = self.load_cached(cached);
        // A failed load can leave part of the file in the graph. The engine cannot rebuild the
        // file, so it reports the file as failed.
        if let Err(error) = loaded {
            self.gate.fail(&error);
            self.cache = None;
            return Err(BuildFailure { message: error });
        }
        let Some(file) = self.graph.get_file(path.as_str()) else {
            let error = format!("cached graph has no file {}", path.as_str());
            self.gate.fail(&error);
            self.cache = None;
            return Err(BuildFailure { message: error });
        };
        if resolve {
            self.files.push((file, path.clone()));
        }
        self.cache_hits += 1;
        Ok(())
    }

    /// Add the graph of one file and its partial paths, from the cache or from a build.
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
        let cache_key = match self.lookup(dialect, path, source, globals) {
            Lookup::Hit(cached) => return self.add_cached(path, &cached, resolve),
            Lookup::Miss { key } => key,
        };
        // The file builds in a stack graph of its own, and the engine then loads the result, as it
        // loads a graph from the cache. A build on a worker thread does the same, so the engine
        // ends up the same for every number of threads, and for a cold and a warm cache.
        let builder = FileBuilder {
            languages: Arc::clone(&self.languages),
            shadow_aware: self.shadow_aware,
        };
        let built = builder.build(dialect, path, source, globals)?;
        self.add_built(path, &built, cache_key.as_ref(), resolve)
    }

    /// Add a file that a [`FileBuilder`] built, after `lookup` gave a miss with `key`. It does
    /// what `add_file` does after a build: it stores the graph in the cache and adds the file.
    /// The caller adds the files in the same order as a serial load, so the engine ends up the
    /// same.
    pub fn add_built(
        &mut self,
        path: &ProjectRelativePath,
        built: &BuiltFile,
        key: Option<&(String, String)>,
        resolve: bool,
    ) -> Result<(), BuildFailure> {
        let fail = |message: String| BuildFailure { message };
        self.load_cached(&built.graph).map_err(fail)?;
        let Some(file) = self.graph.get_file(path.as_str()) else {
            return Err(fail(format!("built graph has no file {}", path.as_str())));
        };
        if built.cacheable
            && let (Some(cache), Some((key, owner))) = (&self.cache, key)
            && let Err(error) = cache.put(key, owner, &built.graph)
        {
            self.gate.fail(&error);
            self.cache = None;
        }
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
        let _ = self.add_partial_paths(file);
        Ok(())
    }

    // The minimal partial paths of a file use no `ResolveLimits`, so the limits stay out of
    // the cache key. A change of the limits cannot make a cached file stale.
    fn add_partial_paths(&mut self, file: Handle<File>) -> Option<Vec<PartialPath>> {
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
            for path in &paths {
                self.database
                    .add_partial_path(&self.graph, &mut self.partials, path.clone());
            }
            Some(paths)
        } else {
            None
        }
    }

    /// Resolve every reference node of every file that built.
    ///
    /// The result lists the files in the order of `add_file`, and the nodes of a file in graph order.
    ///
    /// It also returns the cost of the searches, from the counters of the stitcher.
    ///
    /// With `threads` above 1, the engine freezes its partial paths and resolves the references on
    /// that many threads. Each thread works in its own overlay of the frozen paths. A search
    /// starts from the same state in each overlay, so the result does not depend on `threads`.
    pub fn resolve(
        &mut self,
        limits: ResolveLimits,
        threads: usize,
    ) -> (Vec<RawBinding>, SearchWork) {
        let config = StitcherConfig::default()
            .with_shadow_aware_similar_paths(self.shadow_aware)
            .with_limits(StitcherLimits {
                max_work: Some(limits.max_work),
                max_queue_len: Some(limits.max_queue_len),
                max_arena_cells: Some(limits.max_arena_cells),
                max_path_length: None,
            })
            .with_max_scope_depth(self.max_scope_depth);
        let mut jobs: Vec<(Handle<Node>, SourceAnchor)> = Vec::new();
        for (file, path) in &self.files {
            for node in self.graph.nodes_for_file(*file) {
                if !self.graph[node].is_reference() {
                    continue;
                }
                if let Some(reference) = self.anchor(node, path) {
                    jobs.push((node, reference));
                }
            }
        }
        let graph = &self.graph;
        let database = &self.database;
        let external = &self.external;
        let mut cost = SearchWork::default();
        let mut out = Vec::with_capacity(jobs.len());
        if threads <= 1 || jobs.len() < 2 {
            let partials = &mut self.partials;
            for (node, reference) in jobs {
                let (binding, counters) =
                    resolve_one(graph, partials, database, external, config, node, reference);
                if let Some((work, queue, cells)) = counters {
                    cost.record(work, queue, cells);
                }
                out.push(binding);
            }
            return (out, cost);
        }
        let frozen = std::mem::replace(&mut self.partials, PartialPaths::new()).freeze();
        // The threads take chunks of references in turn. Each result keeps its index, so the
        // merge gives the order of `jobs` whatever thread resolved a reference.
        const CHUNK: usize = 16;
        let next = AtomicUsize::new(0);
        let jobs = &jobs;
        let shared = &frozen;
        let work = || {
            let mut partials = shared.overlay();
            let mut mine = Vec::new();
            loop {
                let start = next.fetch_add(CHUNK, Ordering::Relaxed);
                if start >= jobs.len() {
                    break;
                }
                for (i, (node, reference)) in jobs.iter().enumerate().skip(start).take(CHUNK) {
                    let one = resolve_one(
                        graph,
                        &mut partials,
                        database,
                        external,
                        config,
                        *node,
                        reference.clone(),
                    );
                    mine.push((i, one));
                }
            }
            mine
        };
        let parts: Vec<Vec<(usize, ResolvedOne)>> = std::thread::scope(|scope| {
            let mut workers = Vec::new();
            for _ in 0..threads {
                // A resolve thread needs the stack of a shard worker.
                let builder =
                    std::thread::Builder::new().stack_size(crate::schedule::WORKER_STACK_BYTES);
                match builder.spawn_scoped(scope, work) {
                    Ok(worker) => workers.push(worker),
                    // A thread that cannot start leaves its references to the others.
                    Err(_) => break,
                }
            }
            // No thread started: this thread resolves the references.
            if workers.is_empty() {
                return vec![work()];
            }
            workers
                .into_iter()
                .map(|w| w.join().expect("a resolve thread panicked"))
                .collect()
        });
        let mut slots: Vec<Option<ResolvedOne>> = Vec::new();
        slots.resize_with(jobs.len(), || None);
        for (i, one) in parts.into_iter().flatten() {
            slots[i] = Some(one);
        }
        self.partials = match FrozenPartialPaths::thaw(frozen) {
            Ok(partials) => partials,
            Err(_) => unreachable!("every overlay has dropped"),
        };
        for slot in slots {
            let (binding, counters) = slot.expect("every reference has a result");
            if let Some((work, queue, cells)) = counters {
                cost.record(work, queue, cells);
            }
            out.push(binding);
        }
        (out, cost)
    }

    fn anchor(&self, node: Handle<Node>, path: &ProjectRelativePath) -> Option<SourceAnchor> {
        let info = self.graph.source_info(node)?;
        anchor_from_info(info, path)
    }
}

/// The result of one search: the binding, and the counters of the stitcher when the search
/// returned them.
type ResolvedOne = (RawBinding, Option<(usize, usize, usize)>);

/// Resolve one reference node with bounded work.
fn resolve_one(
    graph: &StackGraph,
    partials: &mut PartialPaths,
    database: &Database,
    external: &HashSet<String>,
    config: StitcherConfig,
    node: Handle<Node>,
    reference: SourceAnchor,
) -> ResolvedOne {
    let reference_name = graph[node].symbol();
    let mut kept: Vec<(PartialPath, RawEnd)> = Vec::new();
    let mut all: BTreeSet<RawEnd> = BTreeSet::new();
    let searched = catch_unwind(AssertUnwindSafe(|| {
        ForwardPartialPathStitcher::find_complete_partial_paths_in_scope(
            graph,
            partials,
            database,
            node,
            config,
            &stack_graphs::NoCancellation,
            |graph, partials, path: &PartialPath| {
                // Drop every path that another path shadows. The engine does not
                // do it. Its own test runner applies the same filter.
                let end = end_of(graph, partials, path, external, reference_name);
                all.insert(end.clone());
                if kept.iter().any(|(other, _)| other.shadows(partials, path)) {
                    return;
                }
                kept.retain(|(other, _)| !path.shadows(partials, other));
                kept.push((path.clone(), end));
            },
        )
    }));
    let mut counters = None;
    let bound = match searched {
        Ok(Ok(stats)) => {
            counters = Some((stats.work, stats.peak_queue_len, stats.arena_cells_added));
            stats.bounded.map(|b| match b {
                StitchBound::Work => Bound::Work,
                StitchBound::QueueLength => Bound::QueueLength,
                StitchBound::ArenaCells => Bound::ArenaCells,
                // The search dropped a long path and went on. It has no path-length
                // limit here, so this value does not occur. Treat it as a failure.
                StitchBound::PathLength => Bound::Failed,
            })
        }
        Ok(Err(_)) | Err(_) => Some(Bound::Failed),
    };
    let ends: BTreeSet<RawEnd> = kept.into_iter().map(|(_, end)| end).collect();
    let shadowed_ends = all.difference(&ends).cloned().collect();
    (
        RawBinding {
            reference,
            ends: ends.into_iter().collect(),
            shadowed_ends,
            bound,
        },
        counters,
    )
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
    reference_name: Option<Handle<Symbol>>,
) -> RawEnd {
    let end = path.end_node;
    let place = place_of(graph, end, external);
    // The edges of the path name every node of the path except the last one.
    let definitions: Vec<Handle<Node>> = path
        .edges
        .iter_unordered(partials)
        .filter_map(|edge| graph.node_for_id(edge.source_node_id))
        .filter(|&node| graph[node].is_definition())
        .collect();
    let places = |keep: &dyn Fn(Handle<Node>) -> bool| {
        let mut found: Vec<Place> = definitions
            .iter()
            .filter(|&&node| keep(node))
            .filter_map(|&node| place_of(graph, node, external))
            .filter(|p| Some(p) != place.as_ref())
            .collect();
        found.sort();
        found.dedup();
        found
    };
    let through = places(&|_| true);
    let echo = places(&|node| reference_name.is_some() && graph[node].symbol() == reference_name);
    RawEnd {
        place,
        syntax_type: graph
            .source_info(end)
            .and_then(|info| info.syntax_type.into_option())
            .map(|h| graph[h].to_string()),
        through,
        echo,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::language::{Builtins, ExternalAsset};

    /// A tiny rule set. Each file defines one symbol, named as the file, and joins it to the root.
    const TSG: &str = r#"global FILE_PATH
global ROOT_NODE
(module) @m {
  node def
  attr (def) type = "pop_symbol", symbol = FILE_PATH, is_definition, source_node = @m
  edge ROOT_NODE -> def
}
"#;

    fn rules(prebuilt: Option<&'static [u8]>) -> BindingRules {
        BindingRules {
            tsg_path: "test.tsg",
            tsg_source: TSG,
            builtins: Some(Builtins {
                path: "builtins.py",
                source: "x = 1\n",
            }),
            assets: vec![ExternalAsset {
                name: "stub.py".to_string(),
                path: "stub.py",
                source: "y = 2\n",
                globals: Vec::new(),
            }],
            shadow_aware_similar_paths: true,
            max_scope_depth: None,
            prebuilt_assets: prebuilt,
        }
    }

    /// Start an engine. Return it with the partial paths that it built itself. The list is empty
    /// when the engine loaded a blob.
    fn start(rules: &BindingRules) -> (BindingEngine, Vec<PartialPath>) {
        let dialects = vec![DialectRules::new(
            tree_sitter_python::LANGUAGE.into(),
            rules.tsg_path,
            rules.tsg_source,
        )];
        let compiled = CompiledRules {
            dialects: Arc::new(dialects),
        };
        let gate = Arc::new(CacheGate::new(
            None,
            std::path::Path::new("."),
            std::time::Duration::ZERO,
            0,
        ));
        BindingEngine::start("test", None, rules, &compiled, &gate, true)
            .expect("the engine starts")
    }

    /// The facts of an engine that must not depend on the way it got its assets.
    fn facts(engine: &BindingEngine) -> (BTreeSet<String>, BTreeSet<String>, usize, usize) {
        let symbols = engine
            .graph
            .iter_symbols()
            .map(|s| engine.graph[s].to_string())
            .collect();
        let external = engine.external.iter().cloned().collect();
        let nodes = engine.graph.iter_nodes().count();
        let paths = engine.database.iter_partial_paths().count();
        (symbols, external, nodes, paths)
    }

    fn leak(bytes: Vec<u8>) -> &'static [u8] {
        Box::leak(bytes.into_boxed_slice())
    }

    fn blob() -> Vec<u8> {
        prebuild_assets(tree_sitter_python::LANGUAGE.into(), &rules(None)).expect("the blob builds")
    }

    #[test]
    fn a_blob_that_matches_the_key_loads() {
        let (built, built_paths) = start(&rules(None));
        assert_eq!(built_paths.len(), 2);
        let (loaded, loaded_paths) = start(&rules(Some(leak(blob()))));
        // The engine built nothing itself: it loaded the blob.
        assert!(loaded_paths.is_empty());
        assert_eq!(facts(&loaded), facts(&built));
        assert!(facts(&loaded).0.contains("<builtins>"));
        assert!(facts(&loaded).0.contains("stub.py"));
    }

    /// An engine with an empty graph, for a call to `load_prebuilt`.
    fn empty_engine() -> BindingEngine {
        BindingEngine {
            pack: "test",
            cache_key: None,
            languages: Arc::new(Vec::new()),
            graph: StackGraph::new(),
            partials: PartialPaths::new(),
            database: Database::new(),
            files: Vec::new(),
            external: HashSet::new(),
            shadow_aware: true,
            max_scope_depth: None,
            cache: None,
            gate: Arc::new(CacheGate::new(
                None,
                std::path::Path::new("."),
                std::time::Duration::ZERO,
                0,
            )),
            cache_hits: 0,
            cache_misses: 0,
        }
    }

    /// Check that the engine rejects the blob, then falls back to the same result as a build.
    fn assert_falls_back(blob: Vec<u8>) {
        let (built, _) = start(&rules(None));
        let rules = rules(Some(leak(blob)));
        let rejected = empty_engine().load_prebuilt(&rules, rules.prebuilt_assets.unwrap());
        assert!(rejected.is_err(), "the blob must not load");
        let (fallback, paths) = start(&rules);
        assert_eq!(paths.len(), 2, "the engine builds the assets itself");
        assert_eq!(facts(&fallback), facts(&built));
    }

    #[test]
    fn a_blob_with_a_wrong_key_falls_back_to_the_build() {
        let mut bad = blob();
        bad[0] = if bad[0] == b'0' { b'1' } else { b'0' };
        assert_falls_back(bad);
    }

    #[test]
    fn a_blob_from_other_rules_falls_back_to_the_build() {
        let mut other = rules(None);
        other.builtins = Some(Builtins {
            path: "builtins.py",
            source: "z = 3\n",
        });
        let blob = prebuild_assets(tree_sitter_python::LANGUAGE.into(), &other).unwrap();
        assert_falls_back(blob);
    }

    #[test]
    fn a_truncated_blob_falls_back_to_the_build() {
        let good = blob();
        let key = assets_key(&rules(None)).len();
        assert_falls_back(good[..key + (good.len() - key) / 2].to_vec());
        assert_falls_back(good[..key].to_vec());
        assert_falls_back(good[..key / 2].to_vec());
        assert_falls_back(Vec::new());
    }

    #[test]
    fn a_blob_with_a_corrupt_body_falls_back_to_the_build() {
        let key = assets_key(&rules(None));
        let mut junk = key.clone().into_bytes();
        junk.extend_from_slice(b"not a compressed graph");
        assert_falls_back(junk);
        let mut flipped = blob();
        for byte in &mut flipped[key.len()..] {
            *byte ^= 0xff;
        }
        assert_falls_back(flipped);
    }

    /// Rules with references. An assignment defines its name at the root. Each identifier is a
    /// reference to its name.
    const REFERENCE_TSG: &str = r#"global FILE_PATH
global ROOT_NODE
(assignment left: (identifier) @n) {
  node def
  attr (def) type = "pop_symbol", symbol = (source-text @n), is_definition, source_node = @n
  edge ROOT_NODE -> def
}
(identifier) @id {
  node ref
  attr (ref) type = "push_symbol", symbol = (source-text @id), is_reference, source_node = @id
  edge ref -> ROOT_NODE
}
"#;

    /// Build an engine over `count` small files. Each file defines two names and refers to
    /// names of the next file and to a name that nobody defines.
    fn engine_with_references(count: usize) -> BindingEngine {
        let mut rules = rules(None);
        rules.tsg_source = REFERENCE_TSG;
        let (mut engine, _) = start(&rules);
        for n in 0..count {
            let next = (n + 1) % count;
            let source = format!("v{n} = 1\nw{n} = v{n}\nprint(v{next}, w{next}, lost{n}, v{n})\n");
            let path = ProjectRelativePath::new(&format!("m{n}.py")).unwrap();
            engine
                .add_file(0, &path, &source, &[], true)
                .expect("the file builds");
        }
        engine
    }

    #[test]
    fn resolving_on_threads_gives_the_bindings_of_one_thread() {
        let mut engine = engine_with_references(40);
        let cells = engine.partials.allocated_cells();
        let (one, one_work) = engine.resolve(ResolveLimits::default(), 1);
        assert!(one.len() > 4 * 16, "the references fill several chunks");
        assert!(
            one.iter().any(|b| !b.ends.is_empty()),
            "some references resolve"
        );
        assert!(
            one.iter().any(|b| b.ends.is_empty()),
            "some references do not"
        );
        assert_eq!(cells, engine.partials.allocated_cells());
        let expected = format!("{one:?}");
        for threads in [2, 4, 7] {
            let (many, many_work) = engine.resolve(ResolveLimits::default(), threads);
            assert_eq!(expected, format!("{many:?}"), "{threads} threads");
            assert_eq!(one_work, many_work, "{threads} threads");
            // The engine takes its partial paths back after the threads end.
            assert_eq!(cells, engine.partials.allocated_cells());
        }
        let (again, _) = engine.resolve(ResolveLimits::default(), 1);
        assert_eq!(expected, format!("{again:?}"));
    }

    #[test]
    fn more_threads_than_references_work() {
        let mut engine = engine_with_references(1);
        let (one, _) = engine.resolve(ResolveLimits::default(), 1);
        let (many, _) = engine.resolve(ResolveLimits::default(), 64);
        assert_eq!(format!("{one:?}"), format!("{many:?}"));
    }
}
