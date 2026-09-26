//! The `candela` command.
//!
//! Two shapes reach the same compiler. `candela file.cdl` compiles and runs one
//! file, which is all a single-file program ever needs. Inside a project, the
//! verbs (`run`, `check`, `build`, `add`, `fetch`, ...) read `candela.toml`,
//! resolve the packages it depends on through `lpm`, and hand the compiler the
//! roots those packages unpacked to.

use crate::Cfg;
use crate::build::BuildOptions;
use crate::build::NativeCode;
use crate::build::build_artifact;
use crate::compiler::compile;
use crate::compiler::imports::ImportResolver;
use crate::errors::BOLD;
use crate::errors::RED;
use crate::errors::RESET;
use crate::execute_compiled;
use crate::lpm;
use crate::manifest::DEFAULT_ENTRY;
use crate::manifest::LOCK_NAME;
use crate::manifest::MANIFEST_NAME;
use crate::manifest::Manifest;
use crate::manifest::new_manifest_text;
use crate::repl::repl;
use crate::trampoline::check_only;
use crate::update;
use crate::util;
use candela_vm::set_argv_skip;
use std::fs;
use std::path::Path;
use std::path::PathBuf;
use std::process::ExitCode;

const USAGE: &str = "Usage:
  candela                          Start the REPL
  candela <file.cdl> [arguments]   Run one file
  candela new <name>               Start a project
  candela run [file.cdl] [args]    Run the project, or the file given
  candela check [file.cdl]         Compile without running
  candela build [file.cdl] [-o out.cdlb] [--debug] [--no-native] [--target <triple>]
                                   Compile to an artifact
  candela add <name>[@requirement] Record a dependency and fetch it
  candela remove <name>            Drop a dependency
  candela fetch [--locked]         Fetch what the manifest lists
  candela update [name...]         Move dependencies to newer versions
  candela publish --url <url>      Publish a release to the registry
  candela [-h | --help]
  candela [-v | --version]

Options:
  --offline   Resolve from the package cache alone (run, check, build, fetch)
  --locked    Refuse to change candela.lock (fetch)
  --debug     Build without the release passes or machine code, as a run from
              source compiles (build)
  --no-native Build without machine code, so the artifact runs on bytecode (build)
  --target <triple>
              Generate the machine code for another machine, such as
              aarch64-apple-darwin (build)
  --cfg <flag | key=value>
              Turn on a flag @cfg(...) tests; repeatable (run, check, build)";

/// Whether an argument names a candela source file, which is how a verb tells
/// the file it was handed from the program's own arguments.
fn is_source(arg: &str) -> bool {
    Path::new(arg)
        .extension()
        .is_some_and(|extension| extension.eq_ignore_ascii_case("cdl"))
}

/// Reports a problem the way every other command-line failure is reported, and
/// leaves.
#[cold]
#[inline(never)]
fn fail(message: &str) -> ! {
    eprintln!("{RED}CANDELA ERROR{RESET}\n{message}");
    std::process::exit(1);
}

/// Reports a command line that does not say what to do.
#[cold]
#[inline(never)]
fn misuse(message: &str) -> ! {
    eprintln!("{RED}CANDELA ERROR{RESET}\n{message}\n{USAGE}");
    std::process::exit(2);
}

/// Runs the command line. Returns the exit status for the shapes that finish
/// normally; everything else leaves through [`fail`] or [`misuse`].
pub fn main() -> ExitCode {
    let mut args = std::env::args().skip(1);

    let Some(first) = args.next() else {
        std::hint::cold_path();
        return repl();
    };

    match first.as_str() {
        "new" => new_project(&mut args),
        "run" => return run_verb(&mut args),
        "check" => check_verb(&mut args),
        "build" | "compile" => build_verb(&mut args),
        "add" => add_verb(&mut args),
        "remove" => remove_verb(&mut args),
        "fetch" => fetch_verb(&mut args),
        "update" => update_verb(&mut args),
        "publish" => publish_verb(&mut args),
        "--help" | "-h" => help(&mut args, &first),
        "--version" | "-v" => version(&mut args, &first),
        _ => run_file(&first, &mut args),
    }
    ExitCode::SUCCESS
}

// ---------------------------------------------------------------------------
// Running and compiling
// ---------------------------------------------------------------------------

/// `candela <file.cdl> [arguments]`: the single-file path, which reads no
/// manifest.
fn run_file(filename: &str, args: &mut impl Iterator<Item = String>) {
    let contents = read_source(Path::new(filename));

    #[cfg(debug_assertions)]
    {
        let next = args.next();
        if next.as_deref() == Some("--debug") {
            let now = std::time::Instant::now();
            let out = compile(contents, filename, true, &ImportResolver::new());
            println!("COMPILATION TIME: {:.2?}", now.elapsed());
            let now = std::time::Instant::now();
            execute_compiled(out);
            println!(
                "EXECUTION TIME: {:.3}ms",
                now.elapsed().as_nanos() / 1_000_000
            );
            return;
        } else if next.as_deref() == Some("--debug-parser") {
            let _ = compile(contents, filename, false, &ImportResolver::new());
            return;
        }
    }
    #[cfg(not(debug_assertions))]
    let _ = args;

    execute_compiled(compile(contents, filename, false, &ImportResolver::new()));
}

/// `candela run [file.cdl] [arguments]`.
///
/// A first argument ending in `.cdl` is the file to run; anything else is the
/// program's own, and the file comes from the manifest.
fn run_verb(args: &mut impl Iterator<Item = String>) -> ExitCode {
    let mut offline = false;
    let mut file = None;
    let mut cfg = Cfg::new();
    // How many arguments after the verb are the command's own. Everything past
    // them belongs to the program.
    let mut own = 0_usize;
    while let Some(arg) = args.next() {
        if arg == "--offline" {
            offline = true;
            own += 1;
            continue;
        }
        if arg == "--cfg" {
            add_cfg(&mut cfg, args.next());
            own += 2;
            continue;
        }
        if is_source(&arg) {
            file = Some(arg);
            own += 1;
        }
        break;
    }

    // `argv()` steps over the binary, the verb, and the command's own
    // arguments, so `candela run a b` reads the same arguments `candela
    // main.cdl a b` does.
    set_argv_skip(2 + own);

    let (path, resolver) = target(file.as_deref(), offline);
    let contents = read_source(&path);
    let out = cfg.scope(|| compile(contents, &path.to_string_lossy(), false, &resolver));
    execute_compiled(out);
    ExitCode::SUCCESS
}

/// `candela check [file.cdl]`: compile and stop.
///
/// The compile is the one `candela build` does, entry points included, so a
/// body error in a function `main` never calls is reported here instead of
/// waiting for the build. It is a check only, so a `dylib` library the
/// program names does not have to exist yet.
fn check_verb(args: &mut impl Iterator<Item = String>) {
    let (file, offline, cfg) = one_file_and_flags(args, "check");
    let (path, resolver) = target(file.as_deref(), offline);
    let contents = read_source(&path);
    let _ = cfg.scope(|| check_only(contents, &path.to_string_lossy(), &resolver));
    println!("{} compiles", path.display());
}

/// `candela build [file.cdl] [-o out.cdlb]`.
fn build_verb(args: &mut impl Iterator<Item = String>) {
    let mut file: Option<String> = None;
    let mut output: Option<String> = None;
    let mut offline = false;
    let mut options = BuildOptions::default();
    let mut cfg = Cfg::new();
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--cfg" => add_cfg(&mut cfg, args.next()),
            "-o" | "--output" => {
                let Some(path) = args.next() else {
                    misuse(&format!("{arg} needs an output path"));
                };
                output = Some(path);
            }
            "--offline" => offline = true,
            "--debug" => options.release = false,
            "--no-native" => options.native = NativeCode::None,
            "--target" => {
                let Some(triple) = args.next() else {
                    misuse("--target needs a target triple, such as aarch64-apple-darwin");
                };
                options.native = NativeCode::Target(triple);
            }
            _ if file.is_none() && is_source(&arg) => file = Some(arg),
            _ => misuse(&format!(
                "unexpected argument {RED}{BOLD}{arg}{RESET}. Name the output file with -o or --output"
            )),
        }
    }

    let (path, resolver) = target(file.as_deref(), offline);
    // A `-o`/`--output` argument is honored verbatim. Otherwise the output name
    // replaces the `.cdl` extension with `.cdlb`, so `program.cdl` becomes
    // `program.cdlb` and never `program.cdl.cdlb`.
    let output = output.unwrap_or_else(|| {
        let text = path.to_string_lossy();
        let stem = text.strip_suffix(".cdl").unwrap_or(&text);
        format!("{stem}.cdlb")
    });

    let contents = read_source(&path);
    let bytes = match cfg
        .scope(|| build_artifact(contents, &path.to_string_lossy(), &resolver, &options))
    {
        Ok(bytes) => bytes,
        Err(e) => fail(&format!("cannot build bytecode: {e}")),
    };
    if let Err(e) = fs::write(&output, &bytes) {
        fail(&format!("cannot write {output}: {e}"));
    }
    println!("Wrote {} ({} bytes)", output, bytes.len());
}

/// Reads a source file, reporting the same way a failed run always has.
fn read_source(path: &Path) -> String {
    fs::read_to_string(path).unwrap_or_else(|_| {
        std::hint::cold_path();
        eprintln!(
            "--------------\n{RED}CANDELA RUNTIME ERROR:{RESET}\nCannot read {RED}{BOLD}{}{RESET}\n--------------",
            path.display()
        );
        std::process::exit(1);
    })
}

/// Settles what a project verb compiles and where its imports read from.
///
/// With a file named on the command line, that file is it. Without one, the
/// manifest's entry is, and the project must have a manifest. Either way, a
/// manifest in scope contributes its dependencies.
fn target(file: Option<&str>, offline: bool) -> (PathBuf, ImportResolver) {
    let cwd = std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."));
    match Manifest::discover(&cwd) {
        Ok(manifest) => {
            let path = file.map_or_else(|| manifest.entry_path(), PathBuf::from);
            if file.is_none() && !path.is_file() {
                fail(&format!(
                    "{} names the entry {}, and there is no file there. Point [package] entry at the file this project runs",
                    manifest.path().display(),
                    manifest.entry()
                ));
            }
            (path, resolve(&manifest, false, offline).0)
        }
        Err(e) => {
            let Some(file) = file else {
                fail(&e.to_string());
            };
            (PathBuf::from(file), ImportResolver::new())
        }
    }
}

/// Resolves the manifest's dependencies and turns them into import roots.
///
/// `lpm` runs only when there is something to resolve, so a project with no
/// dependencies never reaches for it.
fn resolve(
    manifest: &Manifest,
    locked: bool,
    offline: bool,
) -> (ImportResolver, Vec<lpm::Package>) {
    let mut resolver = ImportResolver::new();
    let requirements: Vec<(String, String)> = manifest
        .dependencies()
        .iter()
        .map(|d| (d.name.clone(), d.requirement.clone()))
        .collect();
    if requirements.is_empty() {
        return (resolver, Vec::new());
    }

    let lock = manifest.lock_path();
    let request = lpm::Request {
        lock: &lock,
        requirements: &requirements,
        locked,
        offline,
    };
    let packages = lpm::install(&request).unwrap_or_else(|e| fail(&e.to_string()));
    for package in &packages {
        check_platform(package);
        resolver.add_root(&package.name, package.dir.clone(), package_entry(package));
    }
    (resolver, packages)
}

/// The file `package` is imported by under its own name: the entry its
/// manifest names, relative to the package root.
///
/// A package with no manifest enters at the default; one whose manifest cannot
/// be read is refused, since a manifest that says something wrong is not one
/// to guess around.
fn package_entry(package: &lpm::Package) -> String {
    let path = package.dir.join(MANIFEST_NAME);
    if !path.is_file() {
        return String::from(DEFAULT_ENTRY);
    }
    match Manifest::load(&path) {
        Ok(manifest) => manifest.entry().to_owned(),
        Err(e) => fail(&format!("{} {}: {e}", package.name, package.version)),
    }
}

/// Refuses a package built for something other than candela.
///
/// A Lumen package is markup, styles and engine bindings; a candela project has
/// nothing to do with it, and pulling one in would land a tree of files no
/// import can reach.
fn check_platform(package: &lpm::Package) {
    if package.platform == "candela" {
        return;
    }
    if package.platform == "lumen" {
        fail(&format!(
            "{} {} is a Lumen package, and a candela project cannot depend on one. Drop it from [dependencies], or build the project with lumenc",
            package.name, package.version
        ));
    }
    fail(&format!(
        "{} {} is a {} package, and this project can only use candela packages",
        package.name, package.version, package.platform
    ));
}

// ---------------------------------------------------------------------------
// Project verbs
// ---------------------------------------------------------------------------

/// `candela new <name>`.
fn new_project(args: &mut impl Iterator<Item = String>) {
    let Some(name) = args.next() else {
        misuse("new needs a package name");
    };
    if let Some(extra) = args.next() {
        misuse(&format!("new takes one name, got {extra} as well"));
    }
    let root = PathBuf::from(&name);
    if root.exists() {
        fail(&format!("{} is already there", root.display()));
    }
    let package = root
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or(name.as_str())
        .to_owned();

    let src = root.join("src");
    if let Err(e) = fs::create_dir_all(&src) {
        fail(&format!("cannot create {}: {e}", src.display()));
    }
    write_file(&root.join(MANIFEST_NAME), &new_manifest_text(&package));
    write_file(
        &root.join(DEFAULT_ENTRY),
        "fn main() {\n    print(\"Hello, world!\");\n}\n",
    );

    println!("Created {}", root.display());
    println!("  cd {package}");
    println!("  candela run");
}

fn write_file(path: &Path, contents: &str) {
    if let Err(e) = fs::write(path, contents) {
        fail(&format!("cannot write {}: {e}", path.display()));
    }
}

/// `candela add <name>[@requirement]`.
fn add_verb(args: &mut impl Iterator<Item = String>) {
    let Some(spec) = args.next() else {
        misuse("add needs a package name");
    };
    if let Some(extra) = args.next() {
        misuse(&format!("add takes one package, got {extra} as well"));
    }
    let (name, requested) = match spec.split_once('@') {
        Some((name, requirement)) if !requirement.is_empty() => {
            (name.to_owned(), requirement.to_owned())
        }
        _ => (spec.trim_end_matches('@').to_owned(), String::from("*")),
    };
    if name.is_empty() {
        misuse("add needs a package name");
    }

    let mut manifest = open_manifest();
    let mut requirements: Vec<(String, String)> = manifest
        .dependencies()
        .iter()
        .filter(|d| d.name != name)
        .map(|d| (d.name.clone(), d.requirement.clone()))
        .collect();
    requirements.push((name.clone(), requested.clone()));

    let lock = manifest.lock_path();
    let packages = lpm::install(&lpm::Request {
        lock: &lock,
        requirements: &requirements,
        locked: false,
        offline: false,
    })
    .unwrap_or_else(|e| fail(&e.to_string()));

    let resolved = packages
        .iter()
        .find(|p| p.name == name)
        .unwrap_or_else(|| fail(&format!("lpm resolved the project without {name} in it")));
    check_platform(resolved);

    // An unpinned `add` records what it resolved, at the major and minor it
    // landed on, so a later fetch takes patch releases and nothing wider.
    let recorded = if requested == "*" {
        caret_requirement(&resolved.version)
    } else {
        requested
    };
    manifest.set_dependency(&name, &recorded);
    manifest.save().unwrap_or_else(|e| fail(&e.to_string()));
    println!(
        "Added {name} {recorded} ({} {})",
        resolved.name, resolved.version
    );
}

/// `^X.Y` for a resolved `X.Y.Z`.
fn caret_requirement(version: &str) -> String {
    let mut parts = version.trim_start_matches('v').split('.');
    match (parts.next(), parts.next()) {
        (Some(major), Some(minor)) => format!("^{major}.{minor}"),
        (Some(major), None) => format!("^{major}"),
        _ => format!("^{version}"),
    }
}

/// `candela remove <name>`.
fn remove_verb(args: &mut impl Iterator<Item = String>) {
    let Some(name) = args.next() else {
        misuse("remove needs a package name");
    };
    if let Some(extra) = args.next() {
        misuse(&format!("remove takes one package, got {extra} as well"));
    }
    let mut manifest = open_manifest();
    if !manifest.remove_dependency(&name) {
        fail(&format!(
            "{} lists no dependency called {name}",
            manifest.path().display()
        ));
    }
    manifest.save().unwrap_or_else(|e| fail(&e.to_string()));
    // The lock follows the manifest, so what is left is resolved again. With
    // nothing left there is nothing to ask lpm.
    let _ = resolve(&manifest, false, false);
    println!("Removed {name}");
}

/// `candela fetch [--locked] [--offline]`.
fn fetch_verb(args: &mut impl Iterator<Item = String>) {
    let mut locked = false;
    let mut offline = false;
    for arg in args {
        match arg.as_str() {
            "--locked" => locked = true,
            "--offline" => offline = true,
            other => misuse(&format!("fetch does not take {other}")),
        }
    }
    let manifest = open_manifest();
    if manifest.dependencies().is_empty() {
        println!("{} lists no dependencies", manifest.path().display());
        return;
    }
    let (_, packages) = resolve(&manifest, locked, offline);
    for package in &packages {
        println!(
            "{} {} ({}) {}",
            package.name,
            package.version,
            package.target,
            package.dir.display()
        );
    }
}

/// `candela update [name...]`.
fn update_verb(args: &mut impl Iterator<Item = String>) {
    let mut names = Vec::new();
    let mut offline = false;
    for arg in args {
        match arg.as_str() {
            "--offline" => offline = true,
            other if other.starts_with('-') => misuse(&format!("update does not take {other}")),
            other => names.push(other.to_owned()),
        }
    }
    let manifest = open_manifest();
    let requirements: Vec<(String, String)> = manifest
        .dependencies()
        .iter()
        .map(|d| (d.name.clone(), d.requirement.clone()))
        .collect();
    if requirements.is_empty() {
        println!("{} lists no dependencies", manifest.path().display());
        return;
    }
    let lock = manifest.lock_path();
    let packages = lpm::update(
        &lpm::Request {
            lock: &lock,
            requirements: &requirements,
            locked: false,
            offline,
        },
        &names,
    )
    .unwrap_or_else(|e| fail(&e.to_string()));
    for package in &packages {
        check_platform(package);
        println!("{} {}", package.name, package.version);
    }
    println!("{LOCK_NAME} is up to date");
}

/// `candela publish [--url URL]`.
fn publish_verb(args: &mut impl Iterator<Item = String>) {
    let mut url = None;
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--url" => {
                let Some(value) = args.next() else {
                    misuse("--url needs a URL");
                };
                url = Some(value);
            }
            other => match other.strip_prefix("--url=") {
                Some(value) => url = Some(value.to_owned()),
                None => misuse(&format!("publish does not take {other}")),
            },
        }
    }

    let manifest = open_manifest();
    let Some(url) = url else {
        eprintln!(
            "publish needs the URL of the archive a release serves.\n\
             \n\
             Build one with the reusable workflow, which packs the project, uploads it to a\n\
             GitHub release, and publishes the release to the registry for you:\n\
             \n\
             jobs:\n\
             \x20 release:\n\
             \x20   uses: lumen-fx/candela/.github/workflows/build-package.yml@main\n\
             \x20   with:\n\
             \x20     candela-version: {}\n\
             \x20   secrets:\n\
             \x20     lpm-token: ${{{{ secrets.LPM_TOKEN }}}}\n\
             \n\
             To publish an archive you uploaded yourself, name it:\n\
             \x20 candela publish --url https://example.com/{}-{}.tar.gz",
            env!("CARGO_PKG_VERSION"),
            manifest.name(),
            manifest.version()
        );
        std::process::exit(2);
    };

    // A name nobody has claimed is registered first. A name that is already
    // taken is refused, which is the answer this wanted, whoever holds it.
    if let Err(e) = lpm::publish(manifest.name(), manifest.description())
        && !already_registered(&e.to_string())
    {
        fail(&e.to_string());
    }

    let dependencies: Vec<(String, String)> = manifest
        .dependencies()
        .iter()
        .map(|d| (d.name.clone(), d.requirement.clone()))
        .collect();
    let artifacts = vec![(String::from("any"), url)];
    // The release says which candela a user needs. The manifest says so when
    // its author named one, and otherwise the compiler that checked the
    // package is the only evidence there is.
    let host = match manifest.candela_requirement() {
        Some(requirement) => format!("candela@{requirement}"),
        None => format!("candela@>={}", env!("CARGO_PKG_VERSION")),
    };
    if let Err(e) = lpm::release(
        manifest.name(),
        manifest.version(),
        &artifacts,
        &dependencies,
        &host,
        manifest.description(),
    ) {
        fail(&e.to_string());
    }
    println!(
        "Published {} {}, requiring {host}",
        manifest.name(),
        manifest.version()
    );
}

/// Whether the registry turned a `publish` down because the name is already
/// taken, which is the one refusal that is not a problem. The registry answers
/// the same way whoever holds the name.
///
/// The registry answers a taken name with `package name is already taken`, and
/// `lpm` prints that message as its own. When the body is not the JSON `lpm`
/// expects, it falls back to naming the status, which for this refusal is `the
/// registry answered 409`. Those two lines are the whole of it.
///
/// The test has to be this narrow, because anything wider swallows a real
/// failure and publishes a release against a package that was never
/// registered. The same two phrases are what the reusable workflow greps for in
/// `.github/workflows/build-package.yml`, so both agree on what a taken name
/// looks like.
fn already_registered(message: &str) -> bool {
    let lowered = message.to_ascii_lowercase();
    lowered.contains("package name is already taken")
        || lowered.contains("the registry answered 409")
}

fn open_manifest() -> Manifest {
    let cwd = std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."));
    Manifest::discover(&cwd).unwrap_or_else(|e| fail(&e.to_string()))
}

/// Reads the optional file and the flags a one-file verb takes.
fn one_file_and_flags(
    args: &mut impl Iterator<Item = String>,
    verb: &str,
) -> (Option<String>, bool, Cfg) {
    let mut file = None;
    let mut offline = false;
    let mut cfg = Cfg::new();
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--offline" => offline = true,
            "--cfg" => add_cfg(&mut cfg, args.next()),
            _ if file.is_none() && is_source(&arg) => file = Some(arg),
            other => misuse(&format!("{verb} does not take {other}")),
        }
    }
    (file, offline, cfg)
}

/// Reads the argument after `--cfg` into `cfg`: `web` turns on the flag `web`,
/// and `target=web` or `target="web"` the pair `@cfg(target = "web")` tests.
fn add_cfg(cfg: &mut Cfg, arg: Option<String>) {
    let Some(arg) = arg else {
        misuse("--cfg needs a flag, as in --cfg web, or a pair, as in --cfg target=web");
    };
    match arg.split_once('=') {
        Some((key, value)) => {
            let value = value.trim();
            let value = value
                .strip_prefix('"')
                .and_then(|v| v.strip_suffix('"'))
                .unwrap_or(value);
            cfg.enable_value(key.trim(), value);
        }
        None => cfg.enable(arg.trim()),
    }
}

// ---------------------------------------------------------------------------
// Answers
// ---------------------------------------------------------------------------

fn help(args: &mut impl Iterator<Item = String>, flag: &str) {
    reject_extra_args(args, flag);
    let update = update::start();
    println!(
        "{}\nCandela is a fast, statically-typed compiled scripting language that aims to combine Rust-like syntax with Python's ease-of-use.\n\n{USAGE}\n\nEnvironment:\n  CANDELA_LIB_PATH  The directory holding the shipped std/ library\n  LPM_BIN           The lpm binary to resolve dependencies with, used as given\n",
        util::CANDELA_LOGO
    );
    update::finish(update);
}

fn version(args: &mut impl Iterator<Item = String>, flag: &str) {
    reject_extra_args(args, flag);
    println!("Candela {}", env!("CARGO_PKG_VERSION"));
}

/// Rejects anything trailing `--help` or `--version`.
///
/// Both flags answer a question that takes no further input, so a trailing
/// argument is a mistake; saying so beats printing the answer to a question
/// nobody asked.
fn reject_extra_args(args: &mut impl Iterator<Item = String>, flag: &str) {
    if let Some(extra) = args.next() {
        std::hint::cold_path();
        misuse(&format!(
            "{flag} takes no other arguments, got {RED}{BOLD}{extra}{RESET}"
        ));
    }
}

#[cfg(test)]
mod tests {
    use super::already_registered;
    use super::caret_requirement;

    #[test]
    fn a_resolved_version_records_its_major_and_minor() {
        assert_eq!(caret_requirement("1.2.3"), "^1.2");
        assert_eq!(caret_requirement("0.3.1"), "^0.3");
        assert_eq!(caret_requirement("v2.0.0"), "^2.0");
        assert_eq!(caret_requirement("4"), "^4");
    }

    #[test]
    fn a_taken_name_is_read_out_of_the_refusal() {
        // What lpm prints when the registry refuses a name somebody owns.
        assert!(already_registered("lpm: package name is already taken"));
        // And when the refusal arrives without the JSON body lpm expects.
        assert!(already_registered("lpm: the registry answered 409"));
    }

    #[test]
    fn any_other_refusal_is_a_refusal() {
        for message in [
            "network unreachable",
            "lpm: 401 unauthorized",
            // A real failure that happens to carry the word this used to look
            // for. Swallowing one of these publishes a release against a
            // package that was never registered.
            "lpm: open candela.toml: the file does not exist",
            "lpm: the signing token no longer exists",
            "lpm: package shapes already exists",
            // A different conflict. It is not this one, and a publish that
            // hits it has not registered anything.
            "lpm: release version is already taken",
            "lpm: user already exists",
            // The status named, but not the one this is about.
            "lpm: the registry answered 403",
        ] {
            assert!(!already_registered(message), "{message} was swallowed");
        }
    }
}
