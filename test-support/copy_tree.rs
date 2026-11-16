//! Copy a folder tree for a test that edits its own copy of a fixture.

use std::fs;
use std::path::Path;

/// Copy the files and folders under `from` into `to`. The function creates `to` when it is
/// missing.
pub fn copy_tree(from: &Path, to: &Path) {
    fs::create_dir_all(to).unwrap();
    for entry in fs::read_dir(from).unwrap() {
        let entry = entry.unwrap();
        let target = to.join(entry.file_name());
        if entry.file_type().unwrap().is_dir() {
            copy_tree(&entry.path(), &target);
        } else {
            fs::copy(entry.path(), target).unwrap();
        }
    }
}
