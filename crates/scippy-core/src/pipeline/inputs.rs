//! The manifests and the support files of a run.

use crate::language::{ExtraFile, LanguagePack, Manifest};
use crate::project::ProjectRelativePath;
use crate::report::{Diagnostic, Severity, Stage};
use crate::snapshot::AnalysisInput;
use std::collections::{BTreeMap, HashMap};
use std::path::Path;

/// Where the core reads the manifest `name` of `folder`: its virtual path and its disk path.
/// A shard is a virtual folder, so its manifest is an extra file with a real path.
pub(super) fn manifest_location(
    root: &Path,
    name: &str,
    folder: &str,
    real: &HashMap<&str, &Path>,
) -> (String, std::path::PathBuf) {
    let virtual_path = if folder.is_empty() {
        name.to_string()
    } else {
        format!("{folder}/{name}")
    };
    let disk = real
        .get(virtual_path.as_str())
        .map_or_else(|| root.join(&virtual_path), |p| p.to_path_buf());
    (virtual_path, disk)
}

/// Hash the support files of the run: the manifests of the packs in the root and in each shard,
/// and the files that each pack names with [`LanguagePack::support_inputs`]. A file that does
/// not exist gets no hash, so its later appearance counts as a change. The list is sorted.
pub(super) fn read_support_inputs(
    root: &Path,
    languages: &[&dyn LanguagePack],
    shards: &[ProjectRelativePath],
    extra_files: &[ExtraFile],
) -> Vec<AnalysisInput> {
    let real: HashMap<&str, &Path> = extra_files
        .iter()
        .map(|f| (f.path.as_str(), f.real.as_path()))
        .collect();
    // The path of an input maps to the file that holds it, when that file is not below the root.
    let mut inputs: BTreeMap<String, (Option<String>, std::path::PathBuf)> = BTreeMap::new();
    for pack in languages {
        if let Some(name) = pack.analysis().and_then(|a| a.manifest_name()) {
            let folders = std::iter::once("").chain(shards.iter().map(ProjectRelativePath::as_str));
            for folder in folders {
                let (path, disk) = manifest_location(root, name, folder, &real);
                let outside =
                    (!disk.starts_with(root)).then(|| disk.to_string_lossy().into_owned());
                inputs.insert(path, (outside, disk));
            }
        }
        for file in pack.support_inputs(root) {
            let disk = root.join(file);
            match disk.strip_prefix(root) {
                Ok(inside) => {
                    let path = inside
                        .components()
                        .map(|c| c.as_os_str().to_string_lossy())
                        .collect::<Vec<_>>()
                        .join("/");
                    inputs.entry(path).or_insert((None, disk));
                }
                Err(_) => {
                    let path = disk.to_string_lossy().into_owned();
                    inputs.entry(path.clone()).or_insert((Some(path), disk));
                }
            }
        }
    }
    inputs
        .into_iter()
        .map(|(path, (real_path, disk))| AnalysisInput {
            path,
            real_path,
            sha256: std::fs::read(&disk)
                .ok()
                .map(|bytes| crate::snapshot::sha256_hex(&bytes)),
        })
        .collect()
}

/// Read the manifest `name` in the project root and in each shard.
pub(super) fn read_manifests(
    root: &Path,
    name: &str,
    shards: &[ProjectRelativePath],
    extra_files: &[ExtraFile],
) -> (Vec<Manifest>, Vec<Diagnostic>) {
    let real: HashMap<&str, &Path> = extra_files
        .iter()
        .map(|f| (f.path.as_str(), f.real.as_path()))
        .collect();
    let folders = std::iter::once("").chain(shards.iter().map(ProjectRelativePath::as_str));
    let mut manifests = Vec::new();
    let mut problems = Vec::new();
    for folder in folders {
        let (virtual_path, disk) = manifest_location(root, name, folder, &real);
        match std::fs::read_to_string(disk) {
            Ok(text) => manifests.push(Manifest {
                folder: folder.to_string(),
                text,
            }),
            // A folder with no manifest is normal.
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
            Err(e) => problems.push(Diagnostic {
                severity: Severity::Warning,
                stage: Stage::Discover,
                file: Some(virtual_path),
                range: None,
                code: "manifest-error",
                message: format!("ignored: the manifest cannot be read: {e}"),
            }),
        }
    }
    (manifests, problems)
}
