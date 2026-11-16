//! Choose the graph cache file of an `index` run.
//!
//! The order is: the flags, then the `SCIPPY_GRAPH_CACHE` variable, then the default file in
//! the user cache folder. The default lives here and not in the core, so the core API has no
//! cache unless the caller asks for one.

use std::ffi::OsString;
use std::path::{Path, PathBuf};

/// The variable that names a cache file, or `off`.
pub const ENV: &str = "SCIPPY_GRAPH_CACHE";

/// The choice of a run.
#[derive(Debug, PartialEq, Eq)]
pub enum Choice {
    /// The run has no cache.
    Off,
    /// The user named the file with `--graph-cache` or with the variable.
    Explicit(PathBuf),
    /// The file in the user cache folder.
    Default(PathBuf),
    /// No flag, no variable, and no cache folder: the environment has no `XDG_CACHE_HOME` and
    /// no `HOME`.
    NoFolder,
}

/// Read an environment value. An empty value counts as unset.
type Get<'a> = &'a dyn Fn(&str) -> Option<OsString>;

fn value(get: Get, name: &str) -> Option<OsString> {
    get(name).filter(|v| !v.is_empty())
}

/// The default cache file. `XDG_CACHE_HOME` must be an absolute path. Else use `$HOME/.cache`.
pub fn default_path(get: Get) -> Option<PathBuf> {
    let base = value(get, "XDG_CACHE_HOME")
        .map(PathBuf::from)
        .filter(|p| p.is_absolute())
        .or_else(|| {
            value(get, "HOME")
                .map(PathBuf::from)
                .filter(|p| p.is_absolute())
                .map(|home| home.join(".cache"))
        })?;
    Some(base.join("scippy").join("graph-cache.sqlite"))
}

/// Apply the precedence order: flag, then variable, then default.
pub fn choose(flag: Option<PathBuf>, no_cache: bool, get: Get) -> Choice {
    if no_cache {
        return Choice::Off;
    }
    if let Some(path) = flag {
        return Choice::Explicit(path);
    }
    if let Some(text) = value(get, ENV) {
        if text.eq_ignore_ascii_case("off") {
            return Choice::Off;
        }
        return Choice::Explicit(PathBuf::from(text));
    }
    default_path(get).map_or(Choice::NoFolder, Choice::Default)
}

/// The path of the explicit choice, for the check of the output paths.
pub fn explicit(choice: &Choice) -> Option<&Path> {
    match choice {
        Choice::Explicit(path) => Some(path),
        _ => None,
    }
}

/// Return the cache file for the request. Create the folder of the default file. A failure of
/// the default cache prints one warning and gives no file. It never fails the run.
pub fn prepare(choice: Choice) -> Option<PathBuf> {
    match choice {
        Choice::Off => None,
        Choice::Explicit(path) => Some(path),
        Choice::Default(path) => {
            if let Some(folder) = path.parent()
                && let Err(error) = std::fs::create_dir_all(folder)
            {
                eprintln!(
                    "warning: the graph cache folder {} cannot be created: {error}. \
                     The run goes on without the graph cache",
                    folder.display()
                );
                return None;
            }
            Some(path)
        }
        Choice::NoFolder => {
            eprintln!(
                "warning: no graph cache folder: set XDG_CACHE_HOME or HOME, or pass \
                 --graph-cache. The run goes on without the graph cache"
            );
            None
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn env<'a>(pairs: &'a [(&'a str, &'a str)]) -> impl Fn(&str) -> Option<OsString> + 'a {
        move |name| {
            pairs
                .iter()
                .find(|(k, _)| *k == name)
                .map(|(_, v)| OsString::from(v))
        }
    }

    fn default(pairs: &[(&str, &str)]) -> Choice {
        choose(None, false, &env(pairs))
    }

    #[test]
    fn the_default_follows_xdg_then_home() {
        let both = [("XDG_CACHE_HOME", "/x"), ("HOME", "/h")];
        assert_eq!(
            default(&both),
            Choice::Default("/x/scippy/graph-cache.sqlite".into())
        );
        assert_eq!(
            default(&[("HOME", "/h")]),
            Choice::Default("/h/.cache/scippy/graph-cache.sqlite".into())
        );
        // An empty or relative XDG value is not valid.
        assert_eq!(
            default(&[("XDG_CACHE_HOME", ""), ("HOME", "/h")]),
            Choice::Default("/h/.cache/scippy/graph-cache.sqlite".into())
        );
        assert_eq!(
            default(&[("XDG_CACHE_HOME", "rel"), ("HOME", "/h")]),
            Choice::Default("/h/.cache/scippy/graph-cache.sqlite".into())
        );
        assert_eq!(default(&[]), Choice::NoFolder);
    }

    #[test]
    fn the_order_is_flag_then_variable_then_default() {
        let vars = [(ENV, "/v.sqlite"), ("HOME", "/h")];
        assert_eq!(
            choose(Some("/f.sqlite".into()), false, &env(&vars)),
            Choice::Explicit("/f.sqlite".into())
        );
        assert_eq!(default(&vars), Choice::Explicit("/v.sqlite".into()));
        assert_eq!(default(&[(ENV, "off"), ("HOME", "/h")]), Choice::Off);
        assert_eq!(default(&[(ENV, "OFF"), ("HOME", "/h")]), Choice::Off);
        assert_eq!(
            choose(Some("/f.sqlite".into()), false, &env(&[(ENV, "off")])),
            Choice::Explicit("/f.sqlite".into())
        );
        assert_eq!(choose(None, true, &env(&[(ENV, "/v.sqlite")])), Choice::Off);
        // An empty variable is unset.
        assert!(matches!(
            default(&[(ENV, ""), ("HOME", "/h")]),
            Choice::Default(_)
        ));
    }
}
