//! Where an import resolves from.
//!
//! candela has one import form, a quoted path, and three places it can point:
//! the file next to the one doing the importing, the standard library that
//! ships with the toolchain, and a package the project depends on. This module
//! holds the directories that answer the last two, so a compile is told where
//! to look instead of reading the answer out of the environment each time it
//! needs one.

use smol_strc::SmolStr;
use std::path::Path;
use std::path::PathBuf;

/// The entry a package uses when its manifest names none, relative to the
/// package root. The same default `candela new` writes.
pub const DEFAULT_PACKAGE_ENTRY: &str = "src/main.cdl";

/// The directories a library import is looked up in.
///
/// `lib_dir` is the standard library, which every program can reach.
/// `packages` are the packages this program depends on, each under the name
/// the program imports it by. A package shadows the standard library for the
/// name it claims, so `import "shapes/circle";` reaches the `shapes` package
/// whether or not the toolchain ships a module by that name.
#[derive(Clone, Debug)]
pub struct ImportResolver {
    lib_dir: Option<PathBuf>,
    packages: Vec<PackageRoot>,
}

/// One package a program can import from.
///
/// `dir` is the package root, the directory its `candela.toml` sits in, and
/// `entry` is the file the package is imported by under its own name, relative
/// to that root. Imports of a path inside the package read from the entry's
/// directory.
#[derive(Clone, Debug)]
struct PackageRoot {
    name: SmolStr,
    dir: PathBuf,
    entry: PathBuf,
}

impl PackageRoot {
    /// The directory a path inside the package is read from.
    fn source_dir(&self) -> PathBuf {
        match self.entry.parent() {
            Some(parent) if !parent.as_os_str().is_empty() => self.dir.join(parent),
            _ => self.dir.clone(),
        }
    }
}

impl Default for ImportResolver {
    fn default() -> Self {
        Self {
            lib_dir: default_lib_dir(),
            packages: Vec::new(),
        }
    }
}

impl ImportResolver {
    /// A resolver that reaches the standard library and nothing else.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Points library imports at `dir` instead of the shipped location.
    ///
    /// `dir` is the directory that holds `std/`, the same thing
    /// `CANDELA_LIB_PATH` names.
    pub fn set_lib_dir(&mut self, dir: PathBuf) {
        self.lib_dir = Some(dir);
    }

    /// Makes `dir` the root of the package imported as `name`, entered
    /// through `entry`.
    ///
    /// `entry` is the package's entry file relative to `dir`, the one its
    /// manifest names or [`DEFAULT_PACKAGE_ENTRY`] when it names none.
    /// `import "name";` reads that file, and `import "name/x";` reads `x.cdl`
    /// from the entry's directory. Adding a name twice keeps the last
    /// directory and entry given for it.
    pub fn add_root(&mut self, name: &str, dir: PathBuf, entry: impl Into<PathBuf>) {
        let entry = entry.into();
        if let Some(existing) = self.packages.iter_mut().find(|p| p.name == name) {
            existing.dir = dir;
            existing.entry = entry;
            return;
        }
        self.packages.push(PackageRoot {
            name: SmolStr::from(name),
            dir,
            entry,
        });
    }

    /// The standard library directory in effect.
    #[must_use]
    pub fn lib_dir(&self) -> Option<&Path> {
        self.lib_dir.as_deref()
    }

    /// The package names and root directories, in the order they were added.
    #[must_use]
    pub fn roots(&self) -> Vec<(SmolStr, PathBuf)> {
        self.packages
            .iter()
            .map(|p| (p.name.clone(), p.dir.clone()))
            .collect()
    }

    /// The package root directories on their own, which is what a native
    /// library search wants: a package that ships a `.so` under its root has
    /// it found there.
    #[must_use]
    pub fn root_dirs(&self) -> Vec<PathBuf> {
        self.packages.iter().map(|p| p.dir.clone()).collect()
    }

    /// Where the library import `path` reads from, or `None` when there is
    /// nowhere to look.
    ///
    /// `path` carries the `.cdl` the parser appended, so `import "shapes";`
    /// arrives as `shapes.cdl` and `import "shapes/circle";` as
    /// `shapes/circle.cdl`. The leading segment names the package; the rest is
    /// the path inside it, read from the directory of the package's entry, and
    /// a package with no rest reads the entry itself.
    #[must_use]
    pub fn library_path(&self, path: &str) -> Option<PathBuf> {
        let (head, rest) = split_first_segment(path);
        if let Some(package) = self.packages.iter().find(|p| p.name == head) {
            return Some(match rest {
                Some(rest) => package.source_dir().join(rest),
                None => package.dir.join(&package.entry),
            });
        }
        self.lib_dir.as_ref().map(|dir| dir.join(path))
    }
}

/// Splits an import path into the package it might name and the path inside
/// that package, when there is one.
///
/// `shapes/circle.cdl` is the `shapes` package's `circle.cdl`;
/// `shapes.cdl` is the `shapes` package itself.
fn split_first_segment(path: &str) -> (&str, Option<&str>) {
    match path.find(['/', '\\']) {
        Some(sep) => (&path[..sep], Some(&path[sep + 1..])),
        None => (path.strip_suffix(".cdl").unwrap_or(path), None),
    }
}

/// The standard library directory a resolver starts with: `CANDELA_LIB_PATH`
/// when it is set, and `libs/` beside the running executable otherwise, which
/// is where the toolchain installs it.
fn default_lib_dir() -> Option<PathBuf> {
    if let Some(base) = std::env::var_os("CANDELA_LIB_PATH") {
        return Some(PathBuf::from(base));
    }
    std::env::current_exe().ok().map(|exe| {
        exe.canonicalize()
            .unwrap_or(exe)
            .parent()
            .unwrap_or_else(|| Path::new("."))
            .join("libs")
    })
}

#[cfg(test)]
mod tests {
    use super::DEFAULT_PACKAGE_ENTRY;
    use super::ImportResolver;
    use std::path::Path;
    use std::path::PathBuf;

    fn resolver() -> ImportResolver {
        let mut resolver = ImportResolver {
            lib_dir: Some(PathBuf::from("/toolchain/libs")),
            packages: Vec::new(),
        };
        resolver.add_root(
            "shapes",
            PathBuf::from("/cache/shapes/1.2.3"),
            DEFAULT_PACKAGE_ENTRY,
        );
        resolver
    }

    #[test]
    fn a_std_path_reads_from_the_library_directory() {
        assert_eq!(
            resolver().library_path("std/string.cdl").as_deref(),
            Some(Path::new("/toolchain/libs/std/string.cdl"))
        );
    }

    #[test]
    fn a_package_by_its_own_name_reads_its_entry() {
        assert_eq!(
            resolver().library_path("shapes.cdl").as_deref(),
            Some(Path::new("/cache/shapes/1.2.3/src/main.cdl"))
        );
    }

    #[test]
    fn a_path_inside_a_package_reads_from_the_entry_directory() {
        assert_eq!(
            resolver().library_path("shapes/circle.cdl").as_deref(),
            Some(Path::new("/cache/shapes/1.2.3/src/circle.cdl"))
        );
        assert_eq!(
            resolver().library_path("shapes/two/deep.cdl").as_deref(),
            Some(Path::new("/cache/shapes/1.2.3/src/two/deep.cdl"))
        );
    }

    #[test]
    fn a_manifest_entry_moves_both_forms() {
        let mut resolver = resolver();
        resolver.add_root("geom", PathBuf::from("/cache/geom/0.3.1"), "lib/geom.cdl");
        assert_eq!(
            resolver.library_path("geom.cdl").as_deref(),
            Some(Path::new("/cache/geom/0.3.1/lib/geom.cdl"))
        );
        assert_eq!(
            resolver.library_path("geom/arc.cdl").as_deref(),
            Some(Path::new("/cache/geom/0.3.1/lib/arc.cdl"))
        );
    }

    #[test]
    fn an_entry_at_the_root_reads_paths_from_the_root() {
        let mut resolver = resolver();
        resolver.add_root("flat", PathBuf::from("/cache/flat/1.0.0"), "flat.cdl");
        assert_eq!(
            resolver.library_path("flat.cdl").as_deref(),
            Some(Path::new("/cache/flat/1.0.0/flat.cdl"))
        );
        assert_eq!(
            resolver.library_path("flat/more.cdl").as_deref(),
            Some(Path::new("/cache/flat/1.0.0/more.cdl"))
        );
    }

    #[test]
    fn a_package_root_shadows_the_library_directory() {
        let mut resolver = resolver();
        resolver.add_root(
            "std",
            PathBuf::from("/cache/std/9.9.9"),
            DEFAULT_PACKAGE_ENTRY,
        );
        assert_eq!(
            resolver.library_path("std/string.cdl").as_deref(),
            Some(Path::new("/cache/std/9.9.9/src/string.cdl"))
        );
    }

    #[test]
    fn a_root_added_twice_keeps_the_last_directory() {
        let mut resolver = resolver();
        resolver.add_root("shapes", PathBuf::from("/cache/shapes/2.0.0"), "shapes.cdl");
        assert_eq!(resolver.roots().len(), 1);
        assert_eq!(
            resolver.library_path("shapes.cdl").as_deref(),
            Some(Path::new("/cache/shapes/2.0.0/shapes.cdl"))
        );
    }

    #[test]
    fn without_a_library_directory_only_packages_resolve() {
        let mut resolver = ImportResolver {
            lib_dir: None,
            packages: Vec::new(),
        };
        assert_eq!(resolver.library_path("std/string.cdl"), None);
        resolver.add_root(
            "shapes",
            PathBuf::from("/cache/shapes/1.2.3"),
            DEFAULT_PACKAGE_ENTRY,
        );
        assert_eq!(
            resolver.library_path("shapes.cdl").as_deref(),
            Some(Path::new("/cache/shapes/1.2.3/src/main.cdl"))
        );
    }

    #[test]
    fn the_root_directories_come_back_in_the_order_they_went_in() {
        let mut resolver = resolver();
        resolver.add_root(
            "geom",
            PathBuf::from("/cache/geom/0.3.1"),
            DEFAULT_PACKAGE_ENTRY,
        );
        assert_eq!(
            resolver.root_dirs(),
            vec![
                PathBuf::from("/cache/shapes/1.2.3"),
                PathBuf::from("/cache/geom/0.3.1")
            ]
        );
    }
}
