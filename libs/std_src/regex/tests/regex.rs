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
