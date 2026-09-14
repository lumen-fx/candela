//! End-to-end tests for projects and their dependencies.
//!
//! The registry client is stood in for by a stub on `LPM_BIN` that records the
//! arguments it was handed and answers with a fixed report, so these tests
//! exercise the whole path candela owns: reading `candela.toml`, calling the
//! client, turning the report into import roots, compiling against them, and
//! refusing a package built for something else.
//!
//! One test leaves `LPM_BIN` unset and points the download at local files
//! instead, so the path that installs the client is exercised too.

mod common;

use std::path::Path;
use std::path::PathBuf;
use std::process::Command;

fn repo() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
}

/// A unique scratch directory under the system temp dir.
fn scratch_dir(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!(
        "candela_packages_{tag}_{}_{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    std::fs::create_dir_all(&dir).expect("create scratch dir");
    dir
}

/// A package on disk: one `.cdl` file that exports `area`.
fn shapes_package(root: &Path) -> PathBuf {
    let dir = root.join("cache").join("shapes").join("1.2.3");
    std::fs::create_dir_all(&dir).expect("create package dir");
    std::fs::write(
        dir.join("candela.toml"),
        "[package]\nname = \"shapes\"\nversion = \"1.2.3\"\n",
    )
    .expect("write package manifest");
    std::fs::write(
        dir.join("shapes.cdl"),
        "fn area(w, h) {\n    return w * h;\n}\n",
    )
    .expect("write package source");
    dir
}

/// The report the stub answers with: one package, on the platform given.
fn report(name: &str, version: &str, platform: &str, dir: &Path) -> String {
    format!(
        "{{\"schema\":1,\"lock\":\"{}\",\"packages\":[{{\"name\":\"{name}\",\"version\":\"{version}\",\"platform\":\"{platform}\",\"target\":\"any\",\"dir\":\"{}\",\"files\":[\"candela.toml\",\"{name}.cdl\"],\"dependencies\":{{}}}}]}}",
        json_path(&dir.join("candela.lock")),
        json_path(dir)
    )
}

/// A path as a JSON string body. Windows separators are escaped.
fn json_path(path: &Path) -> String {
    path.to_string_lossy().replace('\\', "\\\\")
}

/// Writes a stub registry client into `dir` and returns the path to it.
///
/// It answers `--version` with something new enough, writes every argument it
/// was handed to `args.txt` beside it, and prints the report for anything else.
/// The report is inside the script, so the stub still answers after the
/// download path has copied it somewhere else.
fn write_stub(dir: &Path, report: &str) -> PathBuf {
    #[cfg(not(windows))]
    {
        use std::os::unix::fs::PermissionsExt as _;
        let path = dir.join("lpm-stub");
        std::fs::write(
            &path,
            format!(
                "#!/bin/sh\n\
                 here=$(dirname \"$0\")\n\
                 if [ \"$1\" = \"--version\" ]; then\n\
                 \x20 echo \"lpm version 9.9.9 (stub)\"\n\
                 \x20 exit 0\n\
                 fi\n\
                 printf '%s\\n' \"$*\" >> \"$here/args.txt\"\n\
                 cat <<'REPORT'\n{report}\nREPORT\n"
            ),
        )
        .expect("write the stub");
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755))
            .expect("make the stub executable");
        path
    }

    #[cfg(windows)]
    {
        let path = dir.join("lpm-stub.cmd");
        std::fs::write(
            &path,
            format!(
                "@echo off\r\n\
                 if \"%1\"==\"--version\" (\r\n\
                 echo lpm version 9.9.9 ^(stub^)\r\n\
                 exit /b 0\r\n\
                 )\r\n\
                 echo %*>>\"%~dp0args.txt\"\r\n\
                 echo {report}\r\n"
            ),
        )
        .expect("write the stub");
        path
    }
}

/// A `candela` command in `dir`, with the stub and the checkout's library
/// directory in place.
fn candela(dir: &Path, stub: Option<&Path>) -> Command {
    let mut command = Command::new(env!("CARGO_BIN_EXE_candela"));
    command
        .current_dir(dir)
        .env("CANDELA_LIB_PATH", repo().join("libs"))
        .env("CANDELA_NO_UPDATE_CHECK", "1");
    match stub {
        Some(stub) => command.env("LPM_BIN", stub),
        None => command.env_remove("LPM_BIN"),
    };
    command
}

fn stdout_of(output: &std::process::Output) -> String {
    String::from_utf8_lossy(&output.stdout).into_owned()
}

fn stderr_of(output: &std::process::Output) -> String {
    String::from_utf8_lossy(&output.stderr).into_owned()
}

/// Starting a project, recording a dependency, and running against it.
#[test]
fn a_project_runs_against_the_package_it_depends_on() {
    let root = scratch_dir("run");
    let package = shapes_package(&root);
    let stub = write_stub(&root, &report("shapes", "1.2.3", "candela", &package));

    let new = common::output_with_deadline(
        candela(&root, Some(&stub)).arg("new").arg("demo"),
        "candela new",
    );
    assert!(new.status.success(), "candela new: {}", stderr_of(&new));

    let project = root.join("demo");
    assert!(project.join("candela.toml").is_file());
    assert!(project.join("src").join("main.cdl").is_file());

    // The generated project runs before anything is added to it.
    let hello = common::output_with_deadline(
        candela(&project, Some(&stub)).arg("run"),
        "candela run hello",
    );
    assert!(hello.status.success(), "candela run: {}", stderr_of(&hello));
    assert!(stdout_of(&hello).contains("Hello, world!"));

    let added = common::output_with_deadline(
        candela(&project, Some(&stub)).arg("add").arg("shapes"),
        "candela add",
    );
    assert!(added.status.success(), "candela add: {}", stderr_of(&added));

    // An unpinned add records what it resolved, at the major and minor.
    let manifest = std::fs::read_to_string(project.join("candela.toml")).unwrap();
    assert!(
        manifest.contains("shapes = \"^1.2\""),
        "the manifest must record the resolved requirement: {manifest}"
    );

    std::fs::write(
        project.join("src").join("main.cdl"),
        "import \"shapes\" as shapes;\n\nfn main() {\n    print(shapes::area(3, 4));\n}\n",
    )
    .unwrap();

    let run = common::output_with_deadline(
        candela(&project, Some(&stub)).arg("run"),
        "candela run shapes",
    );
    assert!(run.status.success(), "candela run: {}", stderr_of(&run));
    assert_eq!(stdout_of(&run).trim(), "12");

    // The command the client was handed carries the requirement, the host, and
    // the target, and never the manifest itself.
    let args = std::fs::read_to_string(root.join("args.txt")).unwrap();
    assert!(args.contains("--req shapes@"), "{args}");
    assert!(args.contains("--host candela@"), "{args}");
    assert!(args.contains("--target "), "{args}");
    assert!(args.contains("--json"), "{args}");
    assert!(!args.contains("candela.toml"), "{args}");

    std::fs::remove_dir_all(&root).ok();
}

/// `check` compiles and stops, and `--offline` reaches the client.
#[test]
fn check_compiles_without_running_and_passes_offline_through() {
    let root = scratch_dir("check");
    let package = shapes_package(&root);
    let stub = write_stub(&root, &report("shapes", "1.2.3", "candela", &package));

    common::output_with_deadline(
        candela(&root, Some(&stub)).arg("new").arg("demo"),
        "candela new",
    );
    let project = root.join("demo");
    std::fs::write(
        project.join("candela.toml"),
        "[package]\nname = \"demo\"\nversion = \"0.1.0\"\n\n[dependencies]\nshapes = \"^1.2\"\n",
    )
    .unwrap();
    std::fs::write(
        project.join("src").join("main.cdl"),
        "import \"shapes\" as shapes;\n\nfn main() {\n    print(\"ran \" + str(shapes::area(2, 5)));\n}\n",
    )
    .unwrap();

    let checked = common::output_with_deadline(
        candela(&project, Some(&stub)).arg("check").arg("--offline"),
        "candela check",
    );
    assert!(
        checked.status.success(),
        "candela check: {}",
        stderr_of(&checked)
    );
    // Compiled, not run: the program's own output is absent.
    let said = stdout_of(&checked);
    assert!(said.contains("compiles"), "{said}");
    assert!(!said.contains("ran "), "{said}");

    let args = std::fs::read_to_string(root.join("args.txt")).unwrap();
    assert!(args.contains("--offline"), "{args}");

    std::fs::remove_dir_all(&root).ok();
}

/// An artifact built from a project carries the package, so it runs under the
/// VM-only binary with no source tree and no client.
#[test]
fn a_built_artifact_runs_with_no_source_tree() {
    let candela_bin = env!("CARGO_BIN_EXE_candela");
    let candela_vm = Path::new(candela_bin)
        .parent()
        .unwrap()
        .join(if cfg!(windows) {
            "candela-vm.exe"
        } else {
            "candela-vm"
        });
    if !candela_vm.exists() {
        eprintln!(
            "skipping a_built_artifact_runs_with_no_source_tree: {} not built",
            candela_vm.display()
        );
        return;
    }

    let root = scratch_dir("build");
    let package = shapes_package(&root);
    let stub = write_stub(&root, &report("shapes", "1.2.3", "candela", &package));

    common::output_with_deadline(
        candela(&root, Some(&stub)).arg("new").arg("demo"),
        "candela new",
    );
    let project = root.join("demo");
    std::fs::write(
        project.join("candela.toml"),
        "[package]\nname = \"demo\"\nversion = \"0.1.0\"\n\n[dependencies]\nshapes = \"^1.2\"\n",
    )
    .unwrap();
    std::fs::write(
        project.join("src").join("main.cdl"),
        "import \"shapes\" as shapes;\n\nfn main() {\n    print(shapes::area(6, 7));\n}\n",
    )
    .unwrap();

    let built = common::output_with_deadline(
        candela(&project, Some(&stub))
            .arg("build")
            .arg("-o")
            .arg("demo.cdlb"),
        "candela build",
    );
    assert!(
        built.status.success(),
        "candela build: {}",
        stderr_of(&built)
    );

    // Nothing to fall back to: the sources and the package both go.
    std::fs::remove_dir_all(project.join("src")).unwrap();
    std::fs::remove_dir_all(root.join("cache")).unwrap();

    let mut vm = Command::new(&candela_vm);
    vm.current_dir(&project).arg("demo.cdlb");
    let ran = common::output_with_deadline(&mut vm, "candela-vm run");
    assert!(ran.status.success(), "candela-vm: {}", stderr_of(&ran));
    assert_eq!(stdout_of(&ran).trim(), "42");

    std::fs::remove_dir_all(&root).ok();
}

/// Arguments after the file reach the program whichever way it was started.
#[test]
fn a_program_reads_its_own_arguments_under_every_shape() {
    let root = scratch_dir("argv");
    let stub = write_stub(&root, &report("shapes", "1.2.3", "candela", &root));
    common::output_with_deadline(
        candela(&root, Some(&stub)).arg("new").arg("demo"),
        "candela new",
    );
    let project = root.join("demo");
    let program = "fn main() {\n    let args = argv();\n    print(args.len());\n    for a in args {\n        print(a);\n    }\n}\n";
    std::fs::write(project.join("src").join("main.cdl"), program).unwrap();

    // The manifest entry, with the program's arguments straight after the verb.
    let from_manifest = common::output_with_deadline(
        candela(&project, Some(&stub))
            .arg("run")
            .arg("one")
            .arg("two"),
        "candela run manifest",
    );
    assert!(
        from_manifest.status.success(),
        "candela run: {}",
        stderr_of(&from_manifest)
    );
    assert_eq!(stdout_of(&from_manifest).trim(), "2\none\ntwo");

    // The same program named outright, which puts one more argument in front.
    let from_file = common::output_with_deadline(
        candela(&project, Some(&stub))
            .arg("run")
            .arg("src/main.cdl")
            .arg("one")
            .arg("two"),
        "candela run file",
    );
    assert!(
        from_file.status.success(),
        "candela run: {}",
        stderr_of(&from_file)
    );
    assert_eq!(stdout_of(&from_file).trim(), "2\none\ntwo");

    // And with no verb at all, which is what it has always been.
    let bare = common::output_with_deadline(
        candela(&project, Some(&stub))
            .arg("src/main.cdl")
            .arg("one")
            .arg("two"),
        "candela file",
    );
    assert!(bare.status.success(), "candela: {}", stderr_of(&bare));
    assert_eq!(stdout_of(&bare).trim(), "2\none\ntwo");

    std::fs::remove_dir_all(&root).ok();
}

/// `remove` takes the entry out, and a name the manifest does not list is an
/// error rather than a quiet success.
#[test]
fn remove_drops_the_entry_it_names() {
    let root = scratch_dir("remove");
    let package = shapes_package(&root);
    let stub = write_stub(&root, &report("shapes", "1.2.3", "candela", &package));

    common::output_with_deadline(
        candela(&root, Some(&stub)).arg("new").arg("demo"),
        "candela new",
    );
    let project = root.join("demo");
    std::fs::write(
        project.join("candela.toml"),
        "[package]\nname = \"demo\"\nversion = \"0.1.0\"\n\n# what it needs\n[dependencies]\nshapes = \"^1.2\"\n",
    )
    .unwrap();

    let removed = common::output_with_deadline(
        candela(&project, Some(&stub)).arg("remove").arg("shapes"),
        "candela remove",
    );
    assert!(
        removed.status.success(),
        "candela remove: {}",
        stderr_of(&removed)
    );
    let manifest = std::fs::read_to_string(project.join("candela.toml")).unwrap();
    assert!(!manifest.contains("shapes"), "{manifest}");
    // The edit keeps the file the author's: the comment above the table is
    // still there.
    assert!(manifest.contains("# what it needs"), "{manifest}");

    let again = common::output_with_deadline(
        candela(&project, Some(&stub)).arg("remove").arg("shapes"),
        "candela remove again",
    );
    assert!(!again.status.success(), "removing twice must be an error");
    assert!(
        stderr_of(&again).contains("shapes"),
        "{}",
        stderr_of(&again)
    );

    std::fs::remove_dir_all(&root).ok();
}

/// `update` re-resolves through the client and reports what it settled on.
#[test]
fn update_re_resolves_and_reports() {
    let root = scratch_dir("update");
    let package = shapes_package(&root);
    let stub = write_stub(&root, &report("shapes", "1.2.3", "candela", &package));

    common::output_with_deadline(
        candela(&root, Some(&stub)).arg("new").arg("demo"),
        "candela new",
    );
    let project = root.join("demo");
    std::fs::write(
        project.join("candela.toml"),
        "[package]\nname = \"demo\"\nversion = \"0.1.0\"\n\n[dependencies]\nshapes = \"^1.2\"\n",
    )
    .unwrap();

    let updated = common::output_with_deadline(
        candela(&project, Some(&stub)).arg("update").arg("shapes"),
        "candela update",
    );
    assert!(
        updated.status.success(),
        "candela update: {}",
        stderr_of(&updated)
    );
    assert!(
        stdout_of(&updated).contains("shapes 1.2.3"),
        "{}",
        stdout_of(&updated)
    );

    // The verb and the package name both reach the client.
    let args = std::fs::read_to_string(root.join("args.txt")).unwrap();
    assert!(args.starts_with("update "), "{args}");
    assert!(args.trim_end().ends_with("shapes"), "{args}");

    std::fs::remove_dir_all(&root).ok();
}

/// A candela project cannot depend on a Lumen package, and the refusal names
/// the one it found.
#[test]
fn a_lumen_package_is_refused_by_name() {
    let root = scratch_dir("lumen");
    let package = shapes_package(&root);
    let stub = write_stub(&root, &report("widgets", "2.0.0", "lumen", &package));

    common::output_with_deadline(
        candela(&root, Some(&stub)).arg("new").arg("demo"),
        "candela new",
    );
    let project = root.join("demo");
    std::fs::write(
        project.join("candela.toml"),
        "[package]\nname = \"demo\"\nversion = \"0.1.0\"\n\n[dependencies]\nwidgets = \"2\"\n",
    )
    .unwrap();

    let run =
        common::output_with_deadline(candela(&project, Some(&stub)).arg("run"), "candela run");
    assert!(!run.status.success(), "a Lumen package must be refused");
    let message = stderr_of(&run);
    assert!(message.contains("widgets"), "{message}");
    assert!(message.contains("Lumen"), "{message}");

    std::fs::remove_dir_all(&root).ok();
}

/// A manifest key candela does not know is reported by name rather than
/// ignored.
#[test]
fn an_unknown_manifest_key_is_named() {
    let root = scratch_dir("badkey");
    std::fs::write(
        root.join("candela.toml"),
        "[package]\nname = \"demo\"\nversion = \"0.1.0\"\nauthors = [\"me\"]\n",
    )
    .unwrap();

    let checked = common::output_with_deadline(candela(&root, None).arg("check"), "candela check");
    assert!(!checked.status.success());
    assert!(
        stderr_of(&checked).contains("authors"),
        "{}",
        stderr_of(&checked)
    );

    std::fs::remove_dir_all(&root).ok();
}

/// Outside a project, a verb with no file says where a project comes from.
#[test]
fn a_verb_outside_a_project_says_how_to_start_one() {
    let root = scratch_dir("noproject");
    let ran = common::output_with_deadline(candela(&root, None).arg("run"), "candela run");
    assert!(!ran.status.success());
    let message = stderr_of(&ran);
    assert!(message.contains("candela.toml"), "{message}");
    assert!(message.contains("candela new"), "{message}");

    std::fs::remove_dir_all(&root).ok();
}

/// With no client on the machine, candela downloads one, checks it against the
/// checksums published beside it, and installs it to the shared path.
///
/// The release is served out of local files rather than the network, so this
/// exercises the whole download path with nothing to reach.
#[test]
fn a_missing_client_is_downloaded_and_verified() {
    if !have("curl") || !have("tar") || !(have("sha256sum") || have("shasum")) {
        eprintln!("skipping a_missing_client_is_downloaded_and_verified: no curl, tar or sha256");
        return;
    }
    let root = scratch_dir("download");
    let package = shapes_package(&root);
    let stub = write_stub(&root, &report("shapes", "1.2.3", "candela", &package));

    // The archive the release serves holds the stub under the name the client
    // installs as.
    let staging = root.join("staging");
    std::fs::create_dir_all(&staging).unwrap();
    let binary = staging.join(if cfg!(windows) { "lpm.exe" } else { "lpm" });
    std::fs::copy(&stub, &binary).unwrap();
    #[cfg(not(windows))]
    {
        use std::os::unix::fs::PermissionsExt as _;
        std::fs::set_permissions(&binary, std::fs::Permissions::from_mode(0o755)).unwrap();
    }

    let asset = asset_name("9.9.9");
    let serve = root.join("releases").join("download").join("v9.9.9");
    std::fs::create_dir_all(&serve).unwrap();
    let archive = serve.join(&asset);
    let packed = Command::new("tar")
        .arg(if cfg!(windows) { "-caf" } else { "-czf" })
        .arg(&archive)
        .arg("-C")
        .arg(&staging)
        .arg(".")
        .status()
        .expect("run tar");
    assert!(packed.success(), "packing the release archive failed");

    let sum = sha256_of(&archive);
    std::fs::write(serve.join("checksums.txt"), format!("{sum}  {asset}\n")).unwrap();

    // A project with one dependency, so the client is reached for.
    let project = root.join("demo");
    std::fs::create_dir_all(project.join("src")).unwrap();
    std::fs::write(
        project.join("candela.toml"),
        "[package]\nname = \"demo\"\nversion = \"0.1.0\"\n\n[dependencies]\nshapes = \"^1.2\"\n",
    )
    .unwrap();

    let home = root.join("home");
    std::fs::create_dir_all(&home).unwrap();
    let mut command = candela(&project, None);
    command
        .arg("fetch")
        .env("HOME", &home)
        .env("LOCALAPPDATA", &home)
        .env("PATH", path_without_client())
        .env(
            "CANDELA_LPM_RELEASES_URL",
            format!("file://{}", root.join("releases").display()),
        )
        .env("CANDELA_LPM_TAG", "v9.9.9");
    let fetched = common::output_with_deadline(&mut command, "candela fetch");

    // Resolving the tag, fetching the checksums and the archive, verifying it,
    // unpacking it and installing it to the shared path all happen everywhere.
    let installed = if cfg!(windows) {
        home.join("Programs").join("lpm").join("lpm.exe")
    } else {
        home.join(".local").join("bin").join("lpm")
    };
    assert!(
        installed.is_file(),
        "the client must land at the shared path: {}\n{}",
        installed.display(),
        stderr_of(&fetched)
    );
    assert!(
        stderr_of(&fetched).contains("Installed lpm 9.9.9"),
        "the install must say where it went: {}",
        stderr_of(&fetched)
    );

    // Running what was installed is the one step this fixture cannot reach on
    // Windows: the stand-in is a batch file, and Windows refuses to execute one
    // named `.exe`. The resolution it would have driven is covered by every
    // other test here, each of which reaches the same stand-in through LPM_BIN.
    #[cfg(not(windows))]
    {
        assert!(
            fetched.status.success(),
            "candela fetch: {}\n{}",
            stderr_of(&fetched),
            stdout_of(&fetched)
        );
        assert!(
            stdout_of(&fetched).contains("shapes"),
            "{}",
            stdout_of(&fetched)
        );
    }

    std::fs::remove_dir_all(&root).ok();
}

/// A download whose bytes do not match the published checksum installs nothing.
#[test]
fn a_download_that_fails_its_checksum_installs_nothing() {
    if !have("curl") || !have("tar") || !(have("sha256sum") || have("shasum")) {
        eprintln!("skipping a_download_that_fails_its_checksum_installs_nothing: no tools");
        return;
    }
    let root = scratch_dir("badsum");
    let staging = root.join("staging");
    std::fs::create_dir_all(&staging).unwrap();
    std::fs::write(staging.join("lpm"), "not a client").unwrap();

    let asset = asset_name("9.9.9");
    let serve = root.join("releases").join("download").join("v9.9.9");
    std::fs::create_dir_all(&serve).unwrap();
    Command::new("tar")
        .arg(if cfg!(windows) { "-caf" } else { "-czf" })
        .arg(serve.join(&asset))
        .arg("-C")
        .arg(&staging)
        .arg(".")
        .status()
        .expect("run tar");
    std::fs::write(
        serve.join("checksums.txt"),
        format!("{}  {asset}\n", "0".repeat(64)),
    )
    .unwrap();

    let project = root.join("demo");
    std::fs::create_dir_all(project.join("src")).unwrap();
    std::fs::write(
        project.join("candela.toml"),
        "[package]\nname = \"demo\"\nversion = \"0.1.0\"\n\n[dependencies]\nshapes = \"^1.2\"\n",
    )
    .unwrap();

    let home = root.join("home");
    std::fs::create_dir_all(&home).unwrap();
    let mut command = candela(&project, None);
    command
        .arg("fetch")
        .env("HOME", &home)
        .env("LOCALAPPDATA", &home)
        .env("PATH", path_without_client())
        .env(
            "CANDELA_LPM_RELEASES_URL",
            format!("file://{}", root.join("releases").display()),
        )
        .env("CANDELA_LPM_TAG", "v9.9.9");
    let fetched = common::output_with_deadline(&mut command, "candela fetch");

    assert!(!fetched.status.success(), "a bad checksum must stop it");
    assert!(
        stderr_of(&fetched).contains("checksum"),
        "{}",
        stderr_of(&fetched)
    );
    let installed = if cfg!(windows) {
        home.join("Programs").join("lpm").join("lpm.exe")
    } else {
        home.join(".local").join("bin").join("lpm")
    };
    assert!(!installed.exists(), "nothing must be installed");

    std::fs::remove_dir_all(&root).ok();
}

/// Whether a program is on `PATH`.
fn have(program: &str) -> bool {
    Command::new(program)
        .arg("--version")
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status()
        .is_ok()
}

/// This machine's `PATH` with every directory that holds a registry client
/// taken out.
///
/// A machine that already has one would never reach the download, and the
/// download is what these tests are about. The rest of `PATH` stays, since the
/// download itself runs `curl`, `tar` and `sha256sum` off it.
fn path_without_client() -> std::ffi::OsString {
    let name = if cfg!(windows) { "lpm.exe" } else { "lpm" };
    let path = std::env::var_os("PATH").unwrap_or_default();
    let kept: Vec<PathBuf> = std::env::split_paths(&path)
        .filter(|dir| !dir.join(name).is_file())
        .collect();
    std::env::join_paths(kept).expect("rebuild PATH")
}

/// The release asset this machine asks for, named the way the registry
/// publishes its client.
fn asset_name(version: &str) -> String {
    let os = if cfg!(windows) {
        "windows"
    } else if cfg!(target_os = "macos") {
        "darwin"
    } else {
        "linux"
    };
    let arch = if cfg!(target_arch = "aarch64") {
        "arm64"
    } else {
        "amd64"
    };
    let extension = if cfg!(windows) { "zip" } else { "tar.gz" };
    format!("lpm_{version}_{os}_{arch}.{extension}")
}

fn sha256_of(path: &Path) -> String {
    for (program, args) in [("sha256sum", &[][..]), ("shasum", &["-a", "256"][..])] {
        if let Ok(out) = Command::new(program).args(args).arg(path).output()
            && out.status.success()
            && let Some(hash) = String::from_utf8_lossy(&out.stdout)
                .split_whitespace()
                .next()
        {
            return hash.to_owned();
        }
    }
    panic!("no sha256sum or shasum to hash the fixture with");
}
