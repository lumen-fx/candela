//! The writer on its own: what an archive may name, and what a write that
//! cannot finish reports.

use std::path::{Path, PathBuf};

use candela_std_archive::pack;

/// A fresh directory of this test's own, emptied first so a rerun starts
/// clean.
fn scratch(case: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!(
        "candela-archive-pack-{}-{case}",
        std::process::id()
    ));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("scratch directory");
    dir
}

fn write(path: &Path, body: &str) {
    std::fs::create_dir_all(path.parent().expect("a parent")).expect("parent directory");
    std::fs::write(path, body).expect("write a file");
}

/// A file that climbs out of the base with `..`, and the base itself, name
/// nothing inside the archive, so the write fails before it starts.
#[test]
fn an_entry_must_name_a_file_under_the_base() {
    let dir = scratch("names");
    let base = dir.join("base");
    write(&base.join("a.txt"), "alpha");
    write(&dir.join("outside.txt"), "out");
    let climbing = base.join("..").join("outside.txt");
    let error = pack::create(&dir.join("out.zip"), &[climbing], &base)
        .expect_err("a climbing entry is refused");
    // Windows resolves the `..` when it makes the path absolute, so there
    // the file is plainly outside the base.
    assert!(
        error.contains("climbs out of the base directory")
            || error.contains("is not under the base directory"),
        "{error}"
    );

    let lone = base.join("a.txt");
    let error = pack::create(&dir.join("lone.tar.gz"), std::slice::from_ref(&lone), &lone)
        .expect_err("the base itself names no entry");
    assert!(error.contains("names no file under the base"), "{error}");
    std::fs::remove_dir_all(&dir).ok();
}

/// A link inside a directory is neither a file nor a directory to walk, so it
/// stays out of the archive.
#[cfg(unix)]
#[test]
fn a_link_in_a_directory_is_left_out() {
    let dir = scratch("link");
    let tree = dir.join("tree");
    write(&tree.join("a.txt"), "alpha");
    std::os::unix::fs::symlink(tree.join("a.txt"), tree.join("link")).expect("make a link");
    let written =
        pack::create(&dir.join("out.tgz"), std::slice::from_ref(&tree), &dir).expect("written");
    assert_eq!(written, 1);
    std::fs::remove_dir_all(&dir).ok();
}

/// A destination that cannot be made reports the path in the way.
#[test]
fn a_destination_that_cannot_be_made_is_reported() {
    let dir = scratch("dest");
    write(&dir.join("a.txt"), "alpha");
    let files = [dir.join("a.txt")];
    // A file where the archive's directory would go.
    write(&dir.join("blocker"), "");
    let error = pack::create(&dir.join("blocker").join("out.zip"), &files, &dir)
        .expect_err("no directory under a file");
    assert!(error.contains("blocker"), "{error}");
    // A directory where the archive itself would go.
    std::fs::create_dir_all(dir.join("taken.zip")).unwrap();
    let error =
        pack::create(&dir.join("taken.zip"), &files, &dir).expect_err("no file over a directory");
    assert!(error.contains("taken.zip"), "{error}");
    std::fs::remove_dir_all(&dir).ok();
}

/// A file or directory the writer may not read fails the write and names it.
#[cfg(unix)]
#[test]
fn an_unreadable_input_is_reported() {
    use std::os::unix::fs::PermissionsExt;

    let dir = scratch("unreadable");
    let secret = dir.join("in").join("secret.txt");
    write(&secret, "hidden");
    std::fs::set_permissions(&secret, std::fs::Permissions::from_mode(0o000)).unwrap();
    let locked = dir.join("locked");
    std::fs::create_dir_all(&locked).unwrap();
    std::fs::set_permissions(&locked, std::fs::Permissions::from_mode(0o000)).unwrap();
    let tar = pack::create(&dir.join("out.tar.gz"), std::slice::from_ref(&secret), &dir);
    let walk = pack::create(&dir.join("out.zip"), std::slice::from_ref(&locked), &dir);
    std::fs::set_permissions(&secret, std::fs::Permissions::from_mode(0o644)).unwrap();
    std::fs::set_permissions(&locked, std::fs::Permissions::from_mode(0o755)).unwrap();
    assert!(tar.expect_err("an unreadable file").contains("secret.txt"));
    assert!(
        walk.expect_err("an unreadable directory")
            .contains("locked")
    );
    std::fs::remove_dir_all(&dir).ok();
}

/// gzip of something that is not a file, and gunzip of something that is not
/// gzip, both fail; the half-written output of the second is removed.
#[test]
fn gzip_and_gunzip_report_input_they_cannot_read() {
    let dir = scratch("gzip");
    // Reading the directory fails, or on Windows opening it does.
    let error = pack::gzip(&dir, &dir.join("dir.gz")).expect_err("a directory has no bytes");
    assert!(error.contains(&dir.display().to_string()), "{error}");

    write(&dir.join("plain.txt"), "not gzip at all");
    let back = dir.join("plain.back");
    let error = pack::gunzip(&dir.join("plain.txt"), &back).expect_err("not gzip");
    assert!(error.contains("plain.txt"), "{error}");
    assert!(!back.exists());

    write(&dir.join("a.txt"), "alpha");
    pack::gzip(&dir.join("a.txt"), &dir.join("a.gz")).expect("gzip");
    pack::gunzip(&dir.join("a.gz"), &dir.join("a.back")).expect("gunzip");
    assert_eq!(
        std::fs::read_to_string(dir.join("a.back")).unwrap(),
        "alpha"
    );
    std::fs::remove_dir_all(&dir).ok();
}
