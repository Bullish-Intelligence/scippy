//! Shards: the grouping of files and the dependencies between shards.

use crate::language::ProjectModel;
use crate::project::{FoundFile, ProjectRelativePath};
use std::collections::{HashMap, HashSet};

/// The shards that `shard` depends on, directly or through another dependency, in the order
/// that a breadth-first walk finds them. Only shards with files count.
pub(super) fn dependency_closure(
    model: &dyn ProjectModel,
    shard: &str,
    members: &HashMap<&str, &[usize]>,
) -> Vec<String> {
    let mut found: Vec<String> = Vec::new();
    let mut seen: HashSet<String> = HashSet::from([shard.to_string()]);
    let mut queue = std::collections::VecDeque::from([shard.to_string()]);
    while let Some(next) = queue.pop_front() {
        for dependency in model.dependencies(&next) {
            if members.contains_key(dependency.as_str()) && seen.insert(dependency.clone()) {
                found.push(dependency.clone());
                queue.push_back(dependency);
            }
        }
    }
    found
}

/// The longest shard that holds the path, or `None` for a path outside every shard.
pub(super) fn shard_of<'a>(
    path: &ProjectRelativePath,
    shards: &'a [ProjectRelativePath],
) -> Option<&'a str> {
    let path = path.as_str();
    shards
        .iter()
        .map(ProjectRelativePath::as_str)
        .filter(|shard| {
            path.strip_prefix(shard)
                .is_some_and(|rest| rest.starts_with('/'))
        })
        .max_by_key(|shard| shard.len())
}

/// The files of each shard, as indexes into `files`. The shards come in the order of their
/// first file, and the files of a shard keep their order.
pub(super) fn shard_groups<'a>(
    files: &[&FoundFile],
    shards: &'a [ProjectRelativePath],
) -> Vec<(Option<&'a str>, Vec<usize>)> {
    let mut groups: Vec<(Option<&str>, Vec<usize>)> = Vec::new();
    let mut slot: HashMap<Option<&str>, usize> = HashMap::new();
    for (at, file) in files.iter().enumerate() {
        let shard = shard_of(&file.path, shards);
        let n = *slot.entry(shard).or_insert_with(|| {
            groups.push((shard, Vec::new()));
            groups.len() - 1
        });
        groups[n].1.push(at);
    }
    groups
}

#[cfg(test)]
mod tests {
    use super::*;

    fn path(p: &str) -> ProjectRelativePath {
        ProjectRelativePath::new(p).unwrap()
    }

    #[test]
    fn a_file_goes_to_the_longest_shard_that_holds_it() {
        let shards = [path("code"), path("code/vendor"), path("lib")];
        assert_eq!(shard_of(&path("code/a.py"), &shards), Some("code"));
        assert_eq!(
            shard_of(&path("code/vendor/b.py"), &shards),
            Some("code/vendor")
        );
        assert_eq!(shard_of(&path("codex/c.py"), &shards), None);
        assert_eq!(shard_of(&path("tool.py"), &shards), None);
    }

    #[test]
    fn shards_keep_the_order_of_their_first_file() {
        let shards = [path("a"), path("b")];
        let found: Vec<FoundFile> = ["x.py", "b/1.py", "a/1.py", "b/2.py", "y.py"]
            .iter()
            .map(|p| FoundFile {
                path: path(p),
                pack: 0,
                real: None,
            })
            .collect();
        let files: Vec<&FoundFile> = found.iter().collect();
        assert_eq!(
            shard_groups(&files, &shards),
            [
                (None, vec![0, 4]),
                (Some("b"), vec![1, 3]),
                (Some("a"), vec![2])
            ]
        );
    }
}
