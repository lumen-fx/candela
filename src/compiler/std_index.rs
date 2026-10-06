//! What the standard library modules a program did not import declare, for the
//! diagnostic that names the import a missing name needs.
//!
//! Only an error reads this, so it reads the library directory then rather than
//! keeping a table that would drift from the modules. A module lists its
//! functions and types at the start of a line, which is all the scan relies
//! on.

use smol_strc::SmolStr;
use std::cell::RefCell;
use std::path::PathBuf;

thread_local! {
    /// The library directory the compile on this thread resolves imports
    /// against, which is where a missing name is looked for.
    static LIB_DIR: RefCell<Option<PathBuf>> = const { RefCell::new(None) };
}

/// Records the library directory a compile resolves imports against.
pub fn set_lib_dir(dir: Option<PathBuf>) {
    LIB_DIR.with(|d| *d.borrow_mut() = dir);
}

/// The standard library modules beside the toolchain, as `(name, text)` pairs
/// in name order: `("math", "...")` for `std/math.cdl`.
fn std_modules() -> Vec<(SmolStr, String)> {
    #[cfg(target_arch = "wasm32")]
    {
        Vec::new()
    }
    #[cfg(not(target_arch = "wasm32"))]
    {
        let Some(dir) = LIB_DIR
            .with(|d| d.borrow().clone())
            .map(|dir| dir.join("std"))
        else {
            return Vec::new();
        };
        let Ok(entries) = std::fs::read_dir(dir) else {
            return Vec::new();
        };
        let mut modules: Vec<(SmolStr, String)> = entries
            .filter_map(Result::ok)
            .filter_map(|entry| {
                let path = entry.path();
                if path.extension().and_then(|e| e.to_str()) != Some("cdl") {
                    return None;
                }
                let name = SmolStr::from(path.file_stem()?.to_str()?);
                let text = std::fs::read_to_string(&path).ok()?;
                Some((name, text))
            })
            .collect();
        modules.sort_by(|a, b| a.0.cmp(&b.0));
        modules
    }
}

/// The name a declaration line declares, when the line starts with `keyword`
/// followed by a space: `fn sqrt(x)` declares `sqrt` for the keyword `fn`.
fn declared_name<'a>(line: &'a str, keyword: &str) -> Option<&'a str> {
    let rest = line.strip_prefix(keyword)?.strip_prefix(' ')?;
    let end = rest
        .find(|c: char| !(c.is_alphanumeric() || c == '_'))
        .unwrap_or(rest.len());
    (end > 0).then(|| &rest[..end])
}

/// The standard library module whose top level declares a function, struct or
/// enum called `name`.
#[must_use]
pub fn module_declaring(name: &str) -> Option<SmolStr> {
    std_modules().into_iter().find_map(|(module, text)| {
        text.lines()
            .any(|line| {
                ["fn", "struct", "enum"]
                    .iter()
                    .any(|keyword| declared_name(line, keyword) == Some(name))
            })
            .then_some(module)
    })
}

/// Whether a standard library module called `name` exists.
#[must_use]
pub fn module_exists(name: &str) -> bool {
    std_modules().iter().any(|(module, _)| module == name)
}

#[cfg(test)]
mod tests {
    use super::declared_name;

    #[test]
    fn a_declaration_line_names_what_it_declares() {
        assert_eq!(declared_name("fn sqrt(x) {", "fn"), Some("sqrt"));
        assert_eq!(declared_name("struct Set<T> {", "struct"), Some("Set"));
        assert_eq!(declared_name("impl list {", "impl"), Some("list"));
        assert_eq!(declared_name("fnord()", "fn"), None);
        assert_eq!(declared_name("/// fn x", "fn"), None);
    }
}
