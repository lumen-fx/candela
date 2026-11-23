//! `std/toml` from candela, against the library the test build made.

use candela_std_testkit::{run, scratch, stage};

#[test]
fn a_document_reads_into_maps_and_writes_back() {
    let dir = scratch("toml");
    let libs = stage(&["toml"], &dir);
    let program = r#"
import "std/toml";

host "test" {
    fn seen(text: string);
}

fn start() {
    let doc = toml::parse("title = \"x\"\n[server]\nport = 8080\ntags = [\"a\", \"b\"]\nwhen = 1979-05-27T07:32:00Z\n").unwrap() as {string: any};
    let server = doc["server"] as {string: any};
    test::seen(string(doc["title"]) + " " + string(server["port"]) + " " + string(server["tags"]) + " " + string(server["when"]));
    test::seen(string(toml::parse("= bad")));
    let back = toml::parse(toml::stringify(doc).unwrap()).unwrap() as {string: any};
    test::seen(string((back["server"] as {string: any})["port"]));
    test::seen(string(toml::stringify([1])));
}

fn main() {}
"#;
    let seen = run(&libs, &dir, program, |seen| seen.len() == 4);
    assert_eq!(
        seen,
        [
            "x 8080 [\"a\",\"b\"] 1979-05-27T07:32:00Z",
            "Err(\"toml_invalid\")",
            "8080",
            "Err(\"toml_unrepresentable\")",
        ]
    );
    std::fs::remove_dir_all(&dir).ok();
}

/// Floats and booleans read and write back, and a `null` anywhere in the
/// value, even inside a nested map, has no TOML form.
#[test]
fn floats_and_booleans_round_trip_and_a_null_is_refused() {
    let dir = scratch("toml-kinds");
    let libs = stage(&["toml"], &dir);
    let program = r#"
import "std/toml";
import "std/json";

host "test" {
    fn seen(text: string);
}

fn start() {
    let doc = toml::parse("ratio = 1.5\non = true\n[inner]\noff = false\n").unwrap() as {string: any};
    test::seen(string(doc["ratio"]) + " " + string(doc["on"]));
    let back = toml::parse(toml::stringify(doc).unwrap()).unwrap() as {string: any};
    test::seen(string(back["ratio"]) + " " + string((back["inner"] as {string: any})["off"]));
    test::seen(string(toml::stringify(json::parse("{\"a\": {\"b\": null}}").unwrap())));
}

fn main() {}
"#;
    let seen = run(&libs, &dir, program, |seen| seen.len() == 3);
    assert_eq!(
        seen,
        ["1.5 true", "1.5 false", "Err(\"toml_unrepresentable\")"]
    );
    std::fs::remove_dir_all(&dir).ok();
}
