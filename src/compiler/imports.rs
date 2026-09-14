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

/// The directories a library import is looked up in.
///
/// `lib_dir` is the standard library, which every program can reach.
/// `roots` are the packages this program depends on, each under the name the
/// program imports it by. A package root shadows the standard library for the
/// name it claims, so `import "shapes/circle";` reaches the `shapes` package
/// whether or not the toolchain ships a module by that name.
#[derive(Clone, Debug)]
pub struct ImportResolver {
    lib_dir: Option<PathBuf>,
    roots: Vec<(SmolStr, PathBuf)>,
}

impl Default for ImportResolver {
    fn default() -> Self {
        Self {
            lib_dir: default_lib_dir(),
            roots: Vec::new(),
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

    /// Makes `dir` the root of the package imported as `name`.
    ///
    /// Adding a name twice keeps the last directory given for it.
    pub fn add_root(&mut self, name: &str, dir: PathBuf) {
        if let Some(existing) = self.roots.iter_mut().find(|(n, _)| n == name) {
            existing.1 = dir;
            return;
        }
        self.roots.push((SmolStr::from(name), dir));
    }

    /// The standard library directory in effect.
    #[must_use]
    pub fn lib_dir(&self) -> Option<&Path> {
        self.lib_dir.as_deref()
    }

    /// The package roots, in the order they were added.
    #[must_use]
    pub fn roots(&self) -> &[(SmolStr, PathBuf)] {
        &self.roots
    }

    /// The package root directories on their own, which is what a native
    /// library search wants: a package that ships a `.so` beside its `.cdl`
    /// sources has it found there.
    #[must_use]
    pub fn root_dirs(&self) -> Vec<PathBuf> {
        self.roots.iter().map(|(_, dir)| dir.clone()).collect()
    }

    /// Where the library import `path` reads from, or `None` when there is
    /// nowhere to look.
    ///
    /// `path` carries the `.cdl` the parser appended, so `import "shapes";`
    /// arrives as `shapes.cdl` and `import "shapes/circle";` as
    /// `shapes/circle.cdl`. The leading segment names the package; the rest is
    /// the path inside it, and a package with no rest reads the file named
    /// after the package itself.
    #[must_use]
    pub fn library_path(&self, path: &str) -> Option<PathBuf> {
        let (head, rest) = split_first_segment(path);
        if let Some((_, root)) = self.roots.iter().find(|(name, _)| name == head) {
            return Some(root.join(rest));
        }
        self.lib_dir.as_ref().map(|dir| dir.join(path))
    }
}

/// Splits an import path into the package it might name and the path inside
/// that package.
///
/// `shapes/circle.cdl` is the `shapes` package's `circle.cdl`;
/// `shapes.cdl` is the `shapes` package's own `shapes.cdl`.
fn split_first_segment(path: &str) -> (&str, &str) {
    match path.find(['/', '\\']) {
        Some(sep) => (&path[..sep], &path[sep + 1..]),
        None => (path.strip_suffix(".cdl").unwrap_or(path), path),
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
    use super::ImportResolver;
    use std::path::Path;
    use std::path::PathBuf;

    fn resolver() -> ImportResolver {
        let mut resolver = ImportResolver {
            lib_dir: Some(PathBuf::from("/toolchain/libs")),
            roots: Vec::new(),
        };
        resolver.add_root("shapes", PathBuf::from("/cache/shapes/1.2.3"));
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
    fn a_package_by_its_own_name_reads_the_file_named_after_it() {
        assert_eq!(
            resolver().library_path("shapes.cdl").as_deref(),
            Some(Path::new("/cache/shapes/1.2.3/shapes.cdl"))
        );
    }

    #[test]
    fn a_path_inside_a_package_drops_the_package_segment() {
        assert_eq!(
            resolver().library_path("shapes/circle.cdl").as_deref(),
            Some(Path::new("/cache/shapes/1.2.3/circle.cdl"))
        );
        assert_eq!(
            resolver().library_path("shapes/two/deep.cdl").as_deref(),
            Some(Path::new("/cache/shapes/1.2.3/two/deep.cdl"))
        );
    }

    #[test]
    fn a_package_root_shadows_the_library_directory() {
        let mut resolver = resolver();
        resolver.add_root("std", PathBuf::from("/cache/std/9.9.9"));
        assert_eq!(
            resolver.library_path("std/string.cdl").as_deref(),
            Some(Path::new("/cache/std/9.9.9/string.cdl"))
        );
    }

    #[test]
    fn a_root_added_twice_keeps_the_last_directory() {
        let mut resolver = resolver();
        resolver.add_root("shapes", PathBuf::from("/cache/shapes/2.0.0"));
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
            roots: Vec::new(),
        };
        assert_eq!(resolver.library_path("std/string.cdl"), None);
        resolver.add_root("shapes", PathBuf::from("/cache/shapes/1.2.3"));
        assert_eq!(
            resolver.library_path("shapes.cdl").as_deref(),
            Some(Path::new("/cache/shapes/1.2.3/shapes.cdl"))
        );
    }

    #[test]
    fn the_root_directories_come_back_in_the_order_they_went_in() {
        let mut resolver = resolver();
        resolver.add_root("geom", PathBuf::from("/cache/geom/0.3.1"));
        assert_eq!(
            resolver.root_dirs(),
            vec![
                PathBuf::from("/cache/shapes/1.2.3"),
                PathBuf::from("/cache/geom/0.3.1")
            ]
        );
    }
}
