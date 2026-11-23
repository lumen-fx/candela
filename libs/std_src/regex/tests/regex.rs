//! `std/regex` from candela, against the library the test build made.

use candela_std_testkit::{run, scratch, stage};

#[test]
fn a_pattern_matches_finds_captures_replaces_and_splits() {
    let dir = scratch("regex");
    let libs = stage(&["regex"], &dir);
    let program = r#"
import "std/regex";

host "test" {
    fn seen(text: string);
}

fn start() {
    let digits = regex::compile("[0-9]+").unwrap();
    test::seen(string(digits.is_match("ab1")) + " " + string(digits.is_match("ab")));
    test::seen(string(digits.find("a12b3")) + " " + string(digits.find("x")));
    test::seen(string(digits.find_all("a1 b22 c333")));
    let pair = regex::compile("(?<k>[a-z]+)=([0-9]+)?").unwrap();
    test::seen(string(pair.captures("x: ab=12")) + " " + string(pair.captures("q=")) + " " + string(pair.captures("!")));
    test::seen(pair.replace_all("a=1 b=2", "${k}:$2"));
    test::seen(string(digits.split("a1b22c")));
    test::seen(string(regex::compile("(")));
}

fn main() {}
"#;
    let seen = run(&libs, &dir, program, |seen| seen.len() == 7);
    assert_eq!(
        seen,
        [
            "true false",
            "Some(\"12\") None",
            "[\"1\",\"22\",\"333\"]",
            "Some([\"ab=12\",\"ab\",\"12\"]) Some([\"q=\",\"q\",\"\"]) None",
            "a:1 b:2",
            "[\"a\",\"b\",\"c\"]",
            "Err(\"regex_invalid\")",
        ]
    );
    std::fs::remove_dir_all(&dir).ok();
}

/// More distinct patterns than the library keeps compiled still answer, and
/// a `Regex` written out by hand with a pattern the engine refuses leaves the
/// text whole rather than raising.
#[test]
fn many_patterns_and_a_refused_one_still_answer() {
    let dir = scratch("regex-many");
    let libs = stage(&["regex"], &dir);
    let program = r#"
import "std/regex";

host "test" {
    fn seen(text: string);
}

fn start() {
    let matched = 0;
    for i in 0..300 {
        let re = regex::compile("x" + string(i) + "y").unwrap();
        if re.is_match("ax" + string(i) + "yb") {
            matched += 1;
        }
    }
    test::seen(string(matched));
    let broken = regex::Regex { pattern: "(" };
    test::seen(broken.replace_all("a(b", "-"));
    test::seen(string(broken.split("a(b")));
}

fn main() {}
"#;
    let seen = run(&libs, &dir, program, |seen| seen.len() == 3);
    assert_eq!(seen, ["300", "a(b", "[\"a(b\"]"]);
    std::fs::remove_dir_all(&dir).ok();
}
