//! `candela.toml`, the project manifest.
//!
//! A manifest names the package, says which file the project's verbs run, and
//! lists the packages it depends on. `candela run`, `candela check` and
//! `candela build` read it when no file is given on the command line, and
//! `candela add` and `candela remove` edit it in place.
//!
//! Editing goes through `toml_edit`, so comments, key order, and whitespace
//! survive an edit: the file is the author's, and a tool that rewrites it from
//! scratch loses everything that was not data.

use std::fmt;
use std::path::Path;
use std::path::PathBuf;
use toml_edit::DocumentMut;
use toml_edit::Item;
use toml_edit::Table;
use toml_edit::value;

/// The file a project is found by.
pub const MANIFEST_NAME: &str = "candela.toml";

/// The lock file that sits beside it.
pub const LOCK_NAME: &str = "candela.lock";

/// The entry point a package uses when it names none.
pub const DEFAULT_ENTRY: &str = "src/main.cdl";

/// Anything wrong with a manifest: it is not there, it does not parse, or it
/// says something candela does not understand.
#[derive(Debug)]
pub enum ManifestError {
    /// No `candela.toml` in this directory or any above it.
    NotFound(PathBuf),
    /// The file could not be read.
    Read(PathBuf, std::io::Error),
    /// The file is not valid TOML.
    Parse(PathBuf, String),
    /// The file is valid TOML but says something candela does not understand.
    Invalid(PathBuf, String),
    /// The file could not be written back.
    Write(PathBuf, std::io::Error),
}

impl fmt::Display for ManifestError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::NotFound(dir) => write!(
                f,
                "no {MANIFEST_NAME} in {} or any directory above it. Start a project with: candela new <name>",
                dir.display()
            ),
            Self::Read(path, e) => write!(f, "cannot read {}: {e}", path.display()),
            Self::Parse(path, e) | Self::Invalid(path, e) => {
                write!(f, "{}: {e}", path.display())
            }
            Self::Write(path, e) => write!(f, "cannot write {}: {e}", path.display()),
        }
    }
}

/// One entry of the `[dependencies]` table.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Dependency {
    pub name: String,
    /// The version requirement, as written. candela passes it to `lpm` and
    /// never interprets it.
    pub requirement: String,
}

/// A parsed `candela.toml`, with the document it came from so an edit keeps the
/// file's own shape.
pub struct Manifest {
    path: PathBuf,
    root: PathBuf,
    doc: DocumentMut,
    name: String,
    version: String,
    description: Option<String>,
    entry: String,
    dependencies: Vec<Dependency>,
}

impl Manifest {
    /// Reads the manifest at `path`.
    ///
    /// # Errors
    ///
    /// Returns a [`ManifestError`] when the file cannot be read, does not
    /// parse, or carries a key candela does not know.
    pub fn load(path: &Path) -> Result<Self, ManifestError> {
        let text = std::fs::read_to_string(path)
            .map_err(|e| ManifestError::Read(path.to_path_buf(), e))?;
        Self::from_str_at(&text, path)
    }

    /// Finds the project that `start` is in and reads its manifest.
    ///
    /// The manifest is looked for in `start` and then in each directory above
    /// it, so a command works anywhere inside a project.
    ///
    /// # Errors
    ///
    /// Returns [`ManifestError::NotFound`] when no directory up to the root has
    /// one, and the same errors as [`Manifest::load`] otherwise.
    pub fn discover(start: &Path) -> Result<Self, ManifestError> {
        let mut dir = Some(start);
        while let Some(current) = dir {
            let candidate = current.join(MANIFEST_NAME);
            if candidate.is_file() {
                return Self::load(&candidate);
            }
            dir = current.parent();
        }
        Err(ManifestError::NotFound(start.to_path_buf()))
    }

    /// Parses `text` as the manifest at `path`.
    ///
    /// # Errors
    ///
    /// Returns a [`ManifestError`] when the text does not parse or carries a
    /// key candela does not know.
    pub fn from_str_at(text: &str, path: &Path) -> Result<Self, ManifestError> {
        let doc: DocumentMut = text.parse().map_err(|e: toml_edit::TomlError| {
            ManifestError::Parse(path.to_path_buf(), e.to_string())
        })?;

        let invalid = |message: String| ManifestError::Invalid(path.to_path_buf(), message);

        for key in doc.iter().map(|(key, _)| key) {
            if key != "package" && key != "dependencies" {
                return Err(invalid(format!(
                    "unknown table [{key}]. A manifest has [package] and [dependencies]"
                )));
            }
        }

        let package = doc
            .get("package")
            .and_then(Item::as_table)
            .ok_or_else(|| invalid(String::from("no [package] table")))?;

        for key in package.iter().map(|(key, _)| key) {
            if !matches!(key, "name" | "version" | "description" | "entry") {
                return Err(invalid(format!(
                    "unknown key {key} in [package]. Known keys are name, version, description, entry"
                )));
            }
        }

        let string_key = |table: &Table, key: &str| -> Result<Option<String>, ManifestError> {
            match table.get(key) {
                None => Ok(None),
                Some(item) => item
                    .as_str()
                    .map(|s| Some(s.to_owned()))
                    .ok_or_else(|| invalid(format!("{key} in [package] must be a string"))),
            }
        };

        let name = string_key(package, "name")?
            .ok_or_else(|| invalid(String::from("no name in [package]")))?;
        if name.is_empty() {
            return Err(invalid(String::from("name in [package] is empty")));
        }
        let version = string_key(package, "version")?
            .ok_or_else(|| invalid(String::from("no version in [package]")))?;
        let description = string_key(package, "description")?;
        let entry = string_key(package, "entry")?.unwrap_or_else(|| String::from(DEFAULT_ENTRY));

        let mut dependencies = Vec::new();
        if let Some(table) = doc.get("dependencies") {
            let table = table
                .as_table()
                .ok_or_else(|| invalid(String::from("[dependencies] must be a table")))?;
            for (dep_name, item) in table {
                let requirement = read_requirement(item).ok_or_else(|| {
                    invalid(format!(
                        "dependency {dep_name} must be a version string or a table with a version key"
                    ))
                })?;
                dependencies.push(Dependency {
                    name: dep_name.to_owned(),
                    requirement,
                });
            }
        }

        let root = path
            .parent()
            .filter(|p| !p.as_os_str().is_empty())
            .unwrap_or_else(|| Path::new("."))
            .to_path_buf();

        Ok(Self {
            path: path.to_path_buf(),
            root,
            doc,
            name,
            version,
            description,
            entry,
            dependencies,
        })
    }

    /// The manifest file itself.
    #[must_use]
    pub fn path(&self) -> &Path {
        &self.path
    }

    /// The package name.
    #[must_use]
    pub fn name(&self) -> &str {
        &self.name
    }

    /// The package version.
    #[must_use]
    pub fn version(&self) -> &str {
        &self.version
    }

    /// The one-line description, when the package has one.
    #[must_use]
    pub fn description(&self) -> Option<&str> {
        self.description.as_deref()
    }

    /// The file the project's verbs run, resolved against the project root.
    #[must_use]
    pub fn entry_path(&self) -> PathBuf {
        self.root.join(&self.entry)
    }

    /// The entry as it is written in the manifest.
    #[must_use]
    pub fn entry(&self) -> &str {
        &self.entry
    }

    /// Where the lock file goes.
    #[must_use]
    pub fn lock_path(&self) -> PathBuf {
        self.root.join(LOCK_NAME)
    }

    /// Every `[dependencies]` entry, in the order the file lists them.
    #[must_use]
    pub fn dependencies(&self) -> &[Dependency] {
        &self.dependencies
    }

    /// Records `name` at `requirement`, replacing an entry that is already
    /// there and leaving the rest of the file as it stands.
    pub fn set_dependency(&mut self, name: &str, requirement: &str) {
        let table = self
            .doc
            .entry("dependencies")
            .or_insert_with(|| Item::Table(Table::new()));
        if let Some(table) = table.as_table_mut() {
            table.set_implicit(false);
            match table.get_mut(name).and_then(Item::as_table_like_mut) {
                // A table entry keeps whatever else it carries; only the
                // version moves.
                Some(entry) => {
                    entry.insert("version", value(requirement));
                }
                None => {
                    table[name] = value(requirement);
                }
            }
        }
        match self.dependencies.iter_mut().find(|d| d.name == name) {
            Some(existing) => existing.requirement.replace_range(.., requirement),
            None => self.dependencies.push(Dependency {
                name: name.to_owned(),
                requirement: requirement.to_owned(),
            }),
        }
    }

    /// Drops `name` from `[dependencies]`. Returns whether it was there.
    pub fn remove_dependency(&mut self, name: &str) -> bool {
        let removed = self
            .doc
            .get_mut("dependencies")
            .and_then(Item::as_table_mut)
            .is_some_and(|table| table.remove(name).is_some());
        self.dependencies.retain(|d| d.name != name);
        removed
    }

    /// Writes the manifest back.
    ///
    /// # Errors
    ///
    /// Returns [`ManifestError::Write`] when the file cannot be written.
    pub fn save(&self) -> Result<(), ManifestError> {
        std::fs::write(&self.path, self.doc.to_string())
            .map_err(|e| ManifestError::Write(self.path.clone(), e))
    }
}

/// Reads the version requirement out of a dependency entry, which is either the
/// requirement on its own (`shapes = "1.2"`) or a table carrying it
/// (`shapes = { version = "1.2" }`).
fn read_requirement(item: &Item) -> Option<String> {
    if let Some(text) = item.as_str() {
        return Some(text.to_owned());
    }
    item.as_table_like()?
        .get("version")?
        .as_str()
        .map(ToOwned::to_owned)
}

/// The text of a manifest for a new package.
#[must_use]
pub fn new_manifest_text(name: &str) -> String {
    format!(
        "[package]\n\
         name = \"{name}\"\n\
         version = \"0.1.0\"\n\
         description = \"\"\n\
         \n\
         [dependencies]\n"
    )
}

#[cfg(test)]
mod tests {
    use super::DEFAULT_ENTRY;
    use super::Manifest;
    use super::ManifestError;
    use super::new_manifest_text;
    use std::path::Path;

    fn parse(text: &str) -> Result<Manifest, ManifestError> {
        Manifest::from_str_at(text, Path::new("/project/candela.toml"))
    }

    #[test]
    fn a_minimal_manifest_takes_the_default_entry() {
        let manifest = parse("[package]\nname = \"geom\"\nversion = \"0.3.1\"\n").unwrap();
        assert_eq!(manifest.name(), "geom");
        assert_eq!(manifest.version(), "0.3.1");
        assert_eq!(manifest.description(), None);
        assert_eq!(manifest.entry(), DEFAULT_ENTRY);
        assert_eq!(manifest.entry_path(), Path::new("/project/src/main.cdl"));
        assert_eq!(manifest.lock_path(), Path::new("/project/candela.lock"));
        assert!(manifest.dependencies().is_empty());
    }

    #[test]
    fn an_entry_is_taken_as_written() {
        let manifest =
            parse("[package]\nname = \"geom\"\nversion = \"0.3.1\"\nentry = \"run.cdl\"\n")
                .unwrap();
        assert_eq!(manifest.entry_path(), Path::new("/project/run.cdl"));
    }

    #[test]
    fn both_dependency_spellings_read_the_same() {
        let manifest = parse(
            "[package]\nname = \"a\"\nversion = \"0.1.0\"\n\
             [dependencies]\nshapes = \"1.2\"\ngeom = { version = \"0.3\" }\n",
        )
        .unwrap();
        let deps = manifest.dependencies();
        assert_eq!(deps.len(), 2);
        assert_eq!(deps[0].name, "shapes");
        assert_eq!(deps[0].requirement, "1.2");
        assert_eq!(deps[1].name, "geom");
        assert_eq!(deps[1].requirement, "0.3");
    }

    #[test]
    fn an_unknown_key_is_named() {
        let err = parse("[package]\nname = \"a\"\nversion = \"0.1.0\"\nauthors = [\"me\"]\n")
            .err()
            .unwrap();
        let message = err.to_string();
        assert!(message.contains("authors"), "{message}");
    }

    #[test]
    fn an_unknown_table_is_named() {
        let err = parse("[package]\nname = \"a\"\nversion = \"0.1.0\"\n[build]\njobs = 2\n")
            .err()
            .unwrap();
        let message = err.to_string();
        assert!(message.contains("[build]"), "{message}");
    }

    #[test]
    fn a_manifest_without_a_package_table_says_so() {
        let err = parse("[dependencies]\nshapes = \"1.2\"\n").err().unwrap();
        assert!(err.to_string().contains("[package]"));
    }

    #[test]
    fn a_missing_version_says_so() {
        let err = parse("[package]\nname = \"a\"\n").err().unwrap();
        assert!(err.to_string().contains("version"));
    }

    #[test]
    fn adding_a_dependency_keeps_comments_and_order() {
        let mut manifest = parse(
            "# my project\n[package]\nname = \"a\"\nversion = \"0.1.0\"\n\n\
             [dependencies]\n# geometry\ngeom = \"0.3\"\n",
        )
        .unwrap();
        manifest.set_dependency("shapes", "^1.2");
        let text = manifest.doc.to_string();
        assert!(text.contains("# my project"), "{text}");
        assert!(text.contains("# geometry"), "{text}");
        assert!(text.contains("shapes = \"^1.2\""), "{text}");
    }

    #[test]
    fn adding_a_dependency_twice_moves_the_requirement() {
        let mut manifest = parse(
            "[package]\nname = \"a\"\nversion = \"0.1.0\"\n[dependencies]\nshapes = \"1.0\"\n",
        )
        .unwrap();
        manifest.set_dependency("shapes", "^2.0");
        assert_eq!(manifest.dependencies().len(), 1);
        assert_eq!(manifest.dependencies()[0].requirement, "^2.0");
        assert!(manifest.doc.to_string().contains("shapes = \"^2.0\""));
    }

    #[test]
    fn a_table_dependency_keeps_its_table_when_the_version_moves() {
        let mut manifest = parse(
            "[package]\nname = \"a\"\nversion = \"0.1.0\"\n\
             [dependencies]\nshapes = { version = \"1.0\" }\n",
        )
        .unwrap();
        manifest.set_dependency("shapes", "^2.0");
        let text = manifest.doc.to_string();
        assert!(text.contains("version = \"^2.0\""), "{text}");
        assert!(text.contains('{'), "{text}");
    }

    #[test]
    fn adding_a_dependency_to_a_manifest_without_the_table_writes_one() {
        let mut manifest = parse("[package]\nname = \"a\"\nversion = \"0.1.0\"\n").unwrap();
        manifest.set_dependency("shapes", "^1.2");
        let text = manifest.doc.to_string();
        assert!(text.contains("[dependencies]"), "{text}");
        assert!(text.contains("shapes = \"^1.2\""), "{text}");
    }

    #[test]
    fn removing_reports_whether_it_was_there() {
        let mut manifest = parse(
            "[package]\nname = \"a\"\nversion = \"0.1.0\"\n[dependencies]\nshapes = \"1.0\"\n",
        )
        .unwrap();
        assert!(manifest.remove_dependency("shapes"));
        assert!(!manifest.remove_dependency("shapes"));
        assert!(manifest.dependencies().is_empty());
        assert!(!manifest.doc.to_string().contains("shapes"));
    }

    #[test]
    fn a_new_manifest_parses_and_names_the_package() {
        let manifest = parse(&new_manifest_text("demo")).unwrap();
        assert_eq!(manifest.name(), "demo");
        assert_eq!(manifest.version(), "0.1.0");
        assert!(manifest.dependencies().is_empty());
    }
}
