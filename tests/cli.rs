//! End-to-end tests driving the real binary against real files.

use std::path::{Path, PathBuf};
use std::process::{Command, Output};

const EXE: &str = env!("CARGO_BIN_EXE_intact");

struct Sandbox {
    dir: PathBuf,
}

impl Sandbox {
    fn new(name: &str) -> Sandbox {
        let dir = std::env::temp_dir().join(format!("intact-test-{}-{}", name, std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        Sandbox { dir }
    }

    fn file(&self, name: &str, bytes: &[u8]) -> PathBuf {
        let path = self.dir.join(name);
        std::fs::write(&path, bytes).unwrap();
        path
    }
}

impl Drop for Sandbox {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.dir);
    }
}

fn run(args: &[&str]) -> Output {
    Command::new(EXE)
        .args(args)
        .output()
        .expect("failed to run intact")
}

fn code(out: &Output) -> i32 {
    out.status.code().unwrap_or(-1)
}

fn read(path: &Path) -> Vec<u8> {
    std::fs::read(path).unwrap()
}

fn stdout(out: &Output) -> String {
    String::from_utf8_lossy(&out.stdout).into_owned()
}

// windows-1252 bytes for "café\nrésumé\n"
const LATIN1: &[u8] = b"caf\xE9\nr\xE9sum\xE9\n";

#[test]
fn latin1_file_stays_latin1() {
    let sb = Sandbox::new("latin1");
    let f = sb.file("a.txt", LATIN1);
    let p = f.to_str().unwrap();

    let out = run(&["replace", p, "--find", "café", "--with", "thé"]);
    assert_eq!(code(&out), 0, "{}", String::from_utf8_lossy(&out.stderr));
    assert_eq!(read(&f), b"th\xE9\nr\xE9sum\xE9\n".to_vec());
}

#[test]
fn repeated_edits_do_not_accumulate_mojibake() {
    let sb = Sandbox::new("repeat");
    let f = sb.file("a.txt", LATIN1);
    let p = f.to_str().unwrap();

    for _ in 0..5 {
        assert_eq!(
            code(&run(&["replace", p, "--find", "café", "--with", "café"])),
            0
        );
    }
    assert_eq!(read(&f), LATIN1.to_vec());
}

#[test]
fn inserted_text_is_transcoded_into_the_file_encoding() {
    let sb = Sandbox::new("insert");
    let f = sb.file("a.txt", LATIN1);
    let p = f.to_str().unwrap();

    let out = run(&["insert", p, "--line", "2", "--text", "à côté"]);
    assert_eq!(code(&out), 0, "{}", String::from_utf8_lossy(&out.stderr));
    assert_eq!(
        read(&f),
        b"caf\xE9\n\xE0 c\xF4t\xE9\nr\xE9sum\xE9\n".to_vec()
    );
}

#[test]
fn unmappable_text_is_refused_and_nothing_is_written() {
    let sb = Sandbox::new("unmappable");
    let f = sb.file("a.txt", LATIN1);
    let p = f.to_str().unwrap();

    let out = run(&["replace", p, "--find", "café", "--with", "日本語"]);
    assert_eq!(code(&out), 5);
    assert_eq!(read(&f), LATIN1.to_vec());

    // ... unless a policy is chosen.
    let out = run(&[
        "replace",
        p,
        "--find",
        "café",
        "--with",
        "日本語",
        "--unmappable",
        "xml",
    ]);
    assert_eq!(code(&out), 0, "{}", String::from_utf8_lossy(&out.stderr));
    assert_eq!(
        read(&f),
        b"&#26085;&#26412;&#35486;\nr\xE9sum\xE9\n".to_vec()
    );
}

#[test]
fn ambiguous_match_is_refused() {
    let sb = Sandbox::new("ambiguous");
    let f = sb.file("a.txt", b"x\nx\n");
    let p = f.to_str().unwrap();

    let out = run(&["replace", p, "--find", "x", "--with", "y"]);
    assert_eq!(code(&out), 4);
    assert_eq!(read(&f), b"x\nx\n".to_vec());

    assert_eq!(
        code(&run(&["replace", p, "--find", "x", "--with", "y", "--all"])),
        0
    );
    assert_eq!(read(&f), b"y\ny\n".to_vec());
}

#[test]
fn missing_match_exits_three() {
    let sb = Sandbox::new("nomatch");
    let f = sb.file("a.txt", b"hello\n");
    let out = run(&[
        "replace",
        f.to_str().unwrap(),
        "--find",
        "nope",
        "--with",
        "x",
    ]);
    assert_eq!(code(&out), 3);
}

#[test]
fn crlf_line_endings_survive_and_are_used_for_new_text() {
    let sb = Sandbox::new("crlf");
    let f = sb.file("a.txt", b"one\r\ntwo\r\n");
    let p = f.to_str().unwrap();

    assert_eq!(
        code(&run(&["insert", p, "--after", "1", "--text", "mid"])),
        0
    );
    assert_eq!(read(&f), b"one\r\nmid\r\ntwo\r\n".to_vec());

    assert_eq!(code(&run(&["append", p, "--text", "end"])), 0);
    assert_eq!(read(&f), b"one\r\nmid\r\ntwo\r\nend\r\n".to_vec());
}

#[test]
fn multiline_text_via_escapes() {
    let sb = Sandbox::new("escapes");
    let f = sb.file("a.txt", b"one\r\n");
    let p = f.to_str().unwrap();

    assert_eq!(
        code(&run(&["append", p, "--escapes", "--text", "a\\nb"])),
        0
    );
    assert_eq!(read(&f), b"one\r\na\r\nb\r\n".to_vec());
}

#[test]
fn delete_and_replace_lines() {
    let sb = Sandbox::new("lines");
    let f = sb.file("a.txt", b"1\n2\n3\n4\n5\n");
    let p = f.to_str().unwrap();

    assert_eq!(code(&run(&["delete", p, "--lines", "2:3"])), 0);
    assert_eq!(read(&f), b"1\n4\n5\n".to_vec());

    assert_eq!(
        code(&run(&[
            "replace-lines",
            p,
            "--lines",
            "$",
            "--text",
            "last"
        ])),
        0
    );
    assert_eq!(read(&f), b"1\n4\nlast\n".to_vec());

    assert_eq!(code(&run(&["delete", p, "--lines", "-2:-1"])), 0);
    assert_eq!(read(&f), b"1\n".to_vec());
}

#[test]
fn move_lines_by_every_destination_form() {
    let sb = Sandbox::new("move");
    let f = sb.file("a.txt", b"1\n2\n3\n4\n5\n6\n7\n8\n9\n10\n");
    let p = f.to_str().unwrap();
    let reset = || std::fs::write(&f, b"1\n2\n3\n4\n5\n6\n7\n8\n9\n10\n").unwrap();

    // Destinations name lines as the file is numbered *now*, so --after 7 puts
    // the block below the line that currently reads "7".
    assert_eq!(
        code(&run(&["move-lines", p, "--lines", "3:4", "--after", "7"])),
        0
    );
    assert_eq!(read(&f), b"1\n2\n5\n6\n7\n3\n4\n8\n9\n10\n".to_vec());

    reset();
    assert_eq!(
        code(&run(&["move-lines", p, "--lines", "7:8", "--before", "3"])),
        0
    );
    assert_eq!(read(&f), b"1\n2\n7\n8\n3\n4\n5\n6\n9\n10\n".to_vec());

    // --by K lands the block's first line at a + K either way round.
    reset();
    assert_eq!(
        code(&run(&["move-lines", p, "--lines", "1:2", "--by", "5"])),
        0
    );
    assert_eq!(read(&f), b"3\n4\n5\n6\n7\n1\n2\n8\n9\n10\n".to_vec());

    reset();
    assert_eq!(
        code(&run(&["move-lines", p, "--lines", "7", "--by", "-3"])),
        0
    );
    assert_eq!(read(&f), b"1\n2\n3\n7\n4\n5\n6\n8\n9\n10\n".to_vec());

    // The two ends of the file: --after $ and --before 1.
    reset();
    assert_eq!(
        code(&run(&["move-lines", p, "--lines", "1:2", "--after", "$"])),
        0
    );
    assert_eq!(read(&f), b"3\n4\n5\n6\n7\n8\n9\n10\n1\n2\n".to_vec());

    reset();
    assert_eq!(
        code(&run(&[
            "move-lines",
            p,
            "--lines",
            "-2:-1",
            "--before",
            "1"
        ])),
        0
    );
    assert_eq!(read(&f), b"9\n10\n1\n2\n3\n4\n5\n6\n7\n8\n".to_vec());
}

#[test]
fn move_lines_refuses_a_destination_it_cannot_honour() {
    let sb = Sandbox::new("movebad");
    let f = sb.file("a.txt", b"1\n2\n3\n4\n5\n");
    let p = f.to_str().unwrap();

    // Inside the block: a block cannot be moved into itself.
    assert_eq!(
        code(&run(&["move-lines", p, "--lines", "2:4", "--after", "3"])),
        2
    );
    assert_eq!(
        code(&run(&["move-lines", p, "--lines", "2:4", "--before", "3"])),
        2
    );
    // The whole file has nowhere to go.
    assert_eq!(
        code(&run(&["move-lines", p, "--lines", "1:$", "--after", "2"])),
        2
    );
    // --by past either end is a range error, not a clamp.
    assert_eq!(
        code(&run(&["move-lines", p, "--lines", "2:4", "--by", "9"])),
        6
    );
    assert_eq!(
        code(&run(&["move-lines", p, "--lines", "2:4", "--by", "-5"])),
        6
    );
    assert_eq!(
        code(&run(&["move-lines", p, "--lines", "9", "--after", "1"])),
        6
    );
    // A destination is required, and only one of them.
    assert_eq!(code(&run(&["move-lines", p, "--lines", "2"])), 2);
    assert_eq!(
        code(&run(&["move-lines", p, "--lines", "2", "--by", "0"])),
        2
    );
    assert_eq!(
        code(&run(&[
            "move-lines",
            p,
            "--lines",
            "2",
            "--after",
            "4",
            "--by",
            "1"
        ])),
        2
    );
    // Every one of those wrote nothing.
    assert_eq!(read(&f), b"1\n2\n3\n4\n5\n".to_vec());

    // Naming where the block already is changes nothing, and is not an error:
    // a script that computes a destination may arrive at the current one.
    let out = run(&["move-lines", p, "--lines", "2:4", "--after", "4"]);
    assert_eq!(code(&out), 0);
    assert!(stdout(&out).contains("unchanged"), "{}", stdout(&out));
    assert_eq!(read(&f), b"1\n2\n3\n4\n5\n".to_vec());
}

#[test]
fn move_lines_keeps_encoding_endings_and_final_newline() {
    let sb = Sandbox::new("movekeep");

    // The moved bytes are spliced, not re-encoded from scratch.
    let f = sb.file("latin1.txt", b"caf\xE9\nr\xE9sum\xE9\nx\n");
    assert_eq!(
        code(&run(&[
            "move-lines",
            f.to_str().unwrap(),
            "--lines",
            "1",
            "--after",
            "2"
        ])),
        0
    );
    assert_eq!(read(&f), b"r\xE9sum\xE9\ncaf\xE9\nx\n".to_vec());

    let f = sb.file("crlf.txt", b"1\r\n2\r\n3\r\n4\r\n");
    assert_eq!(
        code(&run(&[
            "move-lines",
            f.to_str().unwrap(),
            "--lines",
            "1",
            "--after",
            "3"
        ])),
        0
    );
    assert_eq!(read(&f), b"2\r\n3\r\n1\r\n4\r\n".to_vec());

    // A file with no final newline keeps having none, whichever end the block
    // is lifted from: the unterminated last line takes a terminator with it on
    // the way up, and gives its own away on the way down.
    let f = sb.file("tail.txt", b"a\nb\nc");
    assert_eq!(
        code(&run(&[
            "move-lines",
            f.to_str().unwrap(),
            "--lines",
            "$",
            "--before",
            "1"
        ])),
        0
    );
    assert_eq!(read(&f), b"c\na\nb".to_vec());

    let f = sb.file("head.txt", b"a\nb\nc");
    assert_eq!(
        code(&run(&[
            "move-lines",
            f.to_str().unwrap(),
            "--lines",
            "1",
            "--after",
            "$"
        ])),
        0
    );
    assert_eq!(read(&f), b"b\nc\na".to_vec());
}

#[test]
fn file_without_trailing_newline_is_respected() {
    let sb = Sandbox::new("notrailing");
    let f = sb.file("a.txt", b"one\ntwo");
    let p = f.to_str().unwrap();

    assert_eq!(
        code(&run(&["replace-lines", p, "--lines", "2", "--text", "TWO"])),
        0
    );
    assert_eq!(read(&f), b"one\nTWO".to_vec());

    // Appending has to start the new line itself.
    assert_eq!(code(&run(&["append", p, "--text", "three"])), 0);
    assert_eq!(read(&f), b"one\nTWO\nthree\n".to_vec());
}

#[test]
fn out_of_range_line_exits_six() {
    let sb = Sandbox::new("range");
    let f = sb.file("a.txt", b"1\n2\n");
    assert_eq!(
        code(&run(&["delete", f.to_str().unwrap(), "--lines", "9"])),
        6
    );
}

#[test]
fn bom_is_preserved() {
    let sb = Sandbox::new("bom");
    let mut bytes = vec![0xEF, 0xBB, 0xBF];
    bytes.extend_from_slice("héllo\n".as_bytes());
    let f = sb.file("a.txt", &bytes);

    assert_eq!(
        code(&run(&[
            "replace",
            f.to_str().unwrap(),
            "--find",
            "héllo",
            "--with",
            "wörld"
        ])),
        0
    );
    let mut expected = vec![0xEF, 0xBB, 0xBF];
    expected.extend_from_slice("wörld\n".as_bytes());
    assert_eq!(read(&f), expected);
}

#[test]
fn utf16_file_round_trips() {
    let sb = Sandbox::new("utf16");
    let mut bytes = vec![0xFF, 0xFE];
    for u in "alpha\nbeta\n".encode_utf16() {
        bytes.extend_from_slice(&u.to_le_bytes());
    }
    let f = sb.file("a.txt", &bytes);

    assert_eq!(
        code(&run(&[
            "replace",
            f.to_str().unwrap(),
            "--find",
            "beta",
            "--with",
            "gämma"
        ])),
        0
    );
    let mut expected = vec![0xFF, 0xFE];
    for u in "alpha\ngämma\n".encode_utf16() {
        expected.extend_from_slice(&u.to_le_bytes());
    }
    assert_eq!(read(&f), expected);
}

#[test]
fn info_reports_detected_encoding() {
    let sb = Sandbox::new("info");
    let f = sb.file("a.txt", LATIN1);
    let out = run(&["--json", "info", f.to_str().unwrap()]);
    assert_eq!(code(&out), 0);
    let v: serde_json::Value = serde_json::from_str(&stdout(&out)).unwrap();
    assert_eq!(v["ok"], true);
    assert_eq!(v["roundtrip_safe"], true);
    assert_eq!(v["lines"], 2);
    assert_eq!(v["eol"], "lf");
    // chardetng picks a single-byte encoding; the exact guess may vary, but the
    // file must round-trip through it.
    assert!(v["encoding"].as_str().unwrap().len() > 2);
}

#[test]
fn explicit_encoding_overrides_detection() {
    let sb = Sandbox::new("explicit");
    let f = sb.file("a.txt", LATIN1);
    let out = run(&[
        "--json",
        "--encoding",
        "windows-1252",
        "info",
        f.to_str().unwrap(),
    ]);
    let v: serde_json::Value = serde_json::from_str(&stdout(&out)).unwrap();
    assert_eq!(v["encoding"], "windows-1252");
    assert_eq!(v["detected_by"], "explicit");
}

#[test]
fn convert_changes_encoding() {
    let sb = Sandbox::new("convert");
    let f = sb.file("a.txt", LATIN1);
    let p = f.to_str().unwrap();

    let out = run(&["--encoding", "windows-1252", "convert", p, "--to", "utf-8"]);
    assert_eq!(code(&out), 0, "{}", String::from_utf8_lossy(&out.stderr));
    assert_eq!(read(&f), "café\nrésumé\n".as_bytes().to_vec());

    // And back again.
    assert_eq!(code(&run(&["convert", p, "--to", "windows-1252"])), 0);
    assert_eq!(read(&f), LATIN1.to_vec());
}

#[test]
fn dry_run_writes_nothing() {
    let sb = Sandbox::new("dryrun");
    let f = sb.file("a.txt", b"one\ntwo\n");
    let out = run(&[
        "--dry-run",
        "replace",
        f.to_str().unwrap(),
        "--find",
        "one",
        "--with",
        "1",
    ]);
    assert_eq!(code(&out), 0);
    assert_eq!(read(&f), b"one\ntwo\n".to_vec());
    assert!(stdout(&out).contains("-one"));
    assert!(stdout(&out).contains("+1"));
}

/// A file with two changes far apart used to render as one block spanning
/// everything between them, with the second change truncated out of sight.
#[test]
fn distant_changes_stay_separate_and_visible() {
    let sb = Sandbox::new("hunks");
    let body: String = (1..=200)
        .map(|i| {
            if i == 5 || i == 195 {
                format!("line {i} TARGET\n")
            } else {
                format!("line {i} filler\n")
            }
        })
        .collect();
    let f = sb.file("big.txt", body.as_bytes());

    let out = run(&[
        "--dry-run",
        "replace",
        f.to_str().unwrap(),
        "--find",
        "TARGET",
        "--with",
        "CHANGED",
        "--all",
    ]);
    assert_eq!(code(&out), 0);
    let text = stdout(&out);
    assert_eq!(
        text.lines().filter(|l| l.starts_with("@@")).count(),
        2,
        "one hunk per change: {text}"
    );
    assert!(text.contains("-line 5 TARGET"), "{text}");
    assert!(
        text.contains("-line 195 TARGET"),
        "the second change must not be truncated away: {text}"
    );
    assert!(
        text.lines().count() < 30,
        "196 unchanged lines must not be printed: {text}"
    );
}

#[test]
fn a_preview_is_an_applicable_patch() {
    let sb = Sandbox::new("patch");
    let f = sb.file("a.txt", b"alpha\nbeta\ngamma\n");
    let out = run(&[
        "--dry-run",
        "--quiet",
        "replace",
        f.to_str().unwrap(),
        "--find",
        "beta",
        "--with",
        "BETA",
    ]);
    assert_eq!(code(&out), 0);
    let path = f.to_str().unwrap();
    // --quiet drops the summary line, leaving the patch and nothing else.
    assert_eq!(
        stdout(&out),
        format!("--- {path}\n+++ {path}\n@@ -1,3 +1,3 @@\n alpha\n-beta\n+BETA\n gamma\n")
    );
}

#[test]
fn show_diff_prints_the_change_and_applies_it() {
    let sb = Sandbox::new("showdiff");
    let f = sb.file("a.txt", b"alpha\nbeta\n");
    let out = run(&[
        "--show-diff",
        "replace",
        f.to_str().unwrap(),
        "--find",
        "beta",
        "--with",
        "BETA",
    ]);
    assert_eq!(code(&out), 0, "{}", String::from_utf8_lossy(&out.stderr));
    assert_eq!(read(&f), b"alpha\nBETA\n".to_vec(), "the edit was applied");
    let text = stdout(&out);
    assert!(text.contains("-beta\n+BETA\n"), "{text}");
    assert!(text.contains("updated"), "{text}");
}

/// Every setting is a flag. An agent typically runs each command in a fresh
/// shell, so a variable set by one invocation is gone by the next; a setting
/// that applies only sometimes is worse than one that never applies.
#[test]
fn the_environment_is_ignored_entirely() {
    let sb = Sandbox::new("noenv");
    let f = sb.file("a.txt", LATIN1);
    let p = f.to_str().unwrap();

    // Neither a diff switch nor an encoding mandate leaks in from the
    // environment, and a value that would once have been rejected as a bad
    // label is now simply not read.
    let out = run_env(
        &[
            ("INTACT_SHOW_DIFF", "1"),
            ("INTACT_ENCODING", "nonsense-9"),
            ("INTACT_NO_GUESS", "1"),
            ("INTACT_EOL", "wobbly"),
            ("INTACT_STRICT_EOL", "1"),
        ],
        &["--json", "info", p],
    );
    assert_eq!(code(&out), 0, "{}", String::from_utf8_lossy(&out.stderr));
    let v: serde_json::Value = serde_json::from_str(&stdout(&out)).unwrap();
    assert_eq!(v["detected_by"], "guessed", "encoding came from the env");

    // An edit under INTACT_SHOW_DIFF prints no diff, and INTACT_NO_GUESS does
    // not block the write to this guessed-encoding file.
    let out = run_env(
        &[("INTACT_SHOW_DIFF", "1"), ("INTACT_NO_GUESS", "1")],
        &["replace", p, "--find", "café", "--with", "thé"],
    );
    assert_eq!(code(&out), 0, "{}", String::from_utf8_lossy(&out.stderr));
    assert!(!stdout(&out).contains("---"), "{}", stdout(&out));
    assert_eq!(read(&f), b"th\xE9\nr\xE9sum\xE9\n".to_vec());
}

#[test]
fn diff_context_is_adjustable() {
    let sb = Sandbox::new("context");
    let f = sb.file("a.txt", b"1\n2\n3\n4\n5\n6\n7\n8\n9\n");
    let p = f.to_str().unwrap();
    let preview = |context: &str| {
        let out = run(&[
            "--dry-run",
            "--diff-context",
            context,
            "replace",
            p,
            "--find",
            "5",
            "--with",
            "FIVE",
        ]);
        assert_eq!(code(&out), 0);
        stdout(&out)
    };

    let text = preview("0");
    assert!(text.contains("@@ -5,1 +5,1 @@"), "{text}");
    let text = preview("2");
    assert!(text.contains("@@ -3,5 +3,5 @@"), "{text}");
}

/// Converting line endings changes every terminator and no line content. The
/// preview must still say something rather than claim a change and show none.
#[test]
fn line_ending_changes_are_visible_in_a_preview() {
    let sb = Sandbox::new("eoldiff");
    let f = sb.file("a.txt", b"alpha\nbeta\ngamma\n");
    let out = run(&[
        "--dry-run",
        "convert",
        f.to_str().unwrap(),
        "--newlines",
        "crlf",
    ]);
    assert_eq!(code(&out), 0, "{}", String::from_utf8_lossy(&out.stderr));
    let text = stdout(&out);
    assert!(
        text.contains("# line endings: lf=3 crlf=0 cr=0 -> lf=0 crlf=3 cr=0"),
        "{text}"
    );
    assert_eq!(read(&f), b"alpha\nbeta\ngamma\n".to_vec(), "wrote nothing");
}

#[test]
fn a_stray_crlf_in_an_lf_file_shows_up() {
    let sb = Sandbox::new("straycrlf");
    let f = sb.file("a.c", b"int main(void)\n{\n}\n");
    let out = run(&[
        "--dry-run",
        "--eol",
        "crlf",
        "append",
        f.to_str().unwrap(),
        "--text",
        "/* done */",
    ]);
    assert_eq!(code(&out), 0);
    let text = stdout(&out);
    assert!(
        text.contains("# line endings: lf=3 crlf=0 cr=0 -> lf=3 crlf=1 cr=0"),
        "{text}"
    );
    assert!(text.contains("+/* done */"), "{text}");
}

#[test]
fn a_change_with_no_visible_diff_explains_itself() {
    let sb = Sandbox::new("invisible");
    let f = sb.file("a.txt", LATIN1);
    let out = run(&[
        "--encoding",
        "windows-1252",
        "--dry-run",
        "convert",
        f.to_str().unwrap(),
        "--to",
        "utf-8",
    ]);
    assert_eq!(code(&out), 0, "{}", String::from_utf8_lossy(&out.stderr));
    let text = stdout(&out);
    assert!(text.contains("would change"), "{text}");
    assert!(text.contains("# no textual change"), "{text}");
    assert_eq!(read(&f), LATIN1.to_vec());
}

#[test]
fn json_reports_the_spans_that_were_edited() {
    let sb = Sandbox::new("spans");
    let f = sb.file("a.txt", b"alpha\nbeta beta\ngamma\n");
    let out = run(&[
        "--json",
        "replace",
        f.to_str().unwrap(),
        "--find",
        "beta",
        "--with",
        "BETA",
        "--all",
    ]);
    assert_eq!(code(&out), 0, "{}", String::from_utf8_lossy(&out.stderr));
    let v: serde_json::Value = serde_json::from_str(&stdout(&out)).unwrap();
    assert_eq!(v["edit_count"], 2);
    let edits = v["edits"].as_array().unwrap();
    assert_eq!(edits.len(), 2);
    assert_eq!(edits[0]["line"], 2);
    assert_eq!(edits[0]["column"], 1);
    assert_eq!(edits[0]["before"], "beta");
    assert_eq!(edits[0]["after"], "BETA");
    assert_eq!(edits[1]["column"], 6);
    // No diff was asked for, so none is attached.
    assert!(v.get("diff").is_none());
}

#[test]
fn json_carries_the_whole_diff() {
    let sb = Sandbox::new("jsondiff");
    let f = sb.file("a.txt", b"alpha\nbeta\n");
    let out = run(&[
        "--json",
        "--dry-run",
        "replace",
        f.to_str().unwrap(),
        "--find",
        "beta",
        "--with",
        "BETA",
    ]);
    assert_eq!(code(&out), 0);
    let v: serde_json::Value = serde_json::from_str(&stdout(&out)).unwrap();
    let diff = v["diff"].as_str().unwrap();
    assert!(diff.contains("@@ -1,2 +1,2 @@"), "{diff}");
    assert!(diff.contains("-beta\n+BETA\n"), "{diff}");
}

#[test]
fn batch_previews_every_operation_in_one_diff() {
    let sb = Sandbox::new("batchdiff");
    let f = sb.file("a.txt", b"one\ntwo\nthree\n");
    let script = sb.file(
        "ops.json",
        br#"{"ops":[{"op":"replace","find":"two","with":"TWO"},{"op":"append","text":"four"}]}"#,
    );
    let out = run(&[
        "--dry-run",
        "batch",
        f.to_str().unwrap(),
        "--script",
        script.to_str().unwrap(),
    ]);
    assert_eq!(code(&out), 0, "{}", String::from_utf8_lossy(&out.stderr));
    let text = stdout(&out);
    assert!(text.contains("-two\n+TWO\n"), "{text}");
    assert!(text.contains("+four"), "{text}");
    assert_eq!(read(&f), b"one\ntwo\nthree\n".to_vec(), "wrote nothing");
}

#[test]
fn a_new_file_diffs_against_dev_null() {
    let sb = Sandbox::new("newfile");
    let path = sb.dir.join("new.txt");
    let out = run(&[
        "--dry-run",
        "create",
        path.to_str().unwrap(),
        "--text",
        "hello",
    ]);
    assert_eq!(code(&out), 0, "{}", String::from_utf8_lossy(&out.stderr));
    let text = stdout(&out);
    assert!(text.contains("--- /dev/null"), "{text}");
    assert!(text.contains("@@ -0,0 +1,1 @@\n+hello"), "{text}");
    assert!(!path.exists(), "wrote nothing");
}

#[test]
fn losing_the_final_newline_is_shown() {
    let sb = Sandbox::new("finalnl");
    let f = sb.file("a.txt", b"alpha\nbeta");
    let out = run(&[
        "--dry-run",
        "append",
        f.to_str().unwrap(),
        "--text",
        "gamma",
    ]);
    assert_eq!(code(&out), 0);
    let text = stdout(&out);
    assert!(text.contains("\\ No newline at end of file"), "{text}");
}

#[test]
fn backup_keeps_the_original() {
    let sb = Sandbox::new("backup");
    let f = sb.file("a.txt", LATIN1);
    assert_eq!(
        code(&run(&[
            "--backup",
            "replace",
            f.to_str().unwrap(),
            "--find",
            "café",
            "--with",
            "the"
        ])),
        0
    );
    let bak = sb.dir.join("a.txt.bak");
    assert_eq!(read(&bak), LATIN1.to_vec());
}

#[cfg(unix)]
#[test]
fn editing_through_a_symlink_writes_the_target() {
    let sb = Sandbox::new("symlink");
    let real = sb.file("real.txt", LATIN1);
    let link = sb.dir.join("link.txt");
    std::os::unix::fs::symlink(&real, &link).unwrap();

    let out = run(&[
        "replace",
        link.to_str().unwrap(),
        "--find",
        "café",
        "--with",
        "thé",
    ]);
    assert_eq!(code(&out), 0, "{}", String::from_utf8_lossy(&out.stderr));

    // The edit lands in the target, and the link is still a link.
    assert_eq!(read(&real), b"th\xE9\nr\xE9sum\xE9\n".to_vec());
    assert!(
        std::fs::symlink_metadata(&link)
            .unwrap()
            .file_type()
            .is_symlink()
    );

    // A chain of links resolves the same way, as does a link into a
    // subdirectory written relative to the link's own location.
    let sub = sb.dir.join("sub");
    std::fs::create_dir(&sub).unwrap();
    let deep = sb.file("sub/deep.txt", b"x\n");
    std::os::unix::fs::symlink("sub/deep.txt", sb.dir.join("first.txt")).unwrap();
    std::os::unix::fs::symlink("first.txt", sb.dir.join("second.txt")).unwrap();
    let second = sb.dir.join("second.txt");
    let out = run(&[
        "replace",
        second.to_str().unwrap(),
        "--find",
        "x",
        "--with",
        "y",
    ]);
    assert_eq!(code(&out), 0, "{}", String::from_utf8_lossy(&out.stderr));
    assert_eq!(read(&deep), b"y\n".to_vec());
    assert!(
        std::fs::symlink_metadata(&second)
            .unwrap()
            .file_type()
            .is_symlink()
    );
}

#[cfg(unix)]
#[test]
fn a_symlink_is_named_in_the_result() {
    let sb = Sandbox::new("symreport");
    let real = sb.file("real.txt", b"hello\n");
    let link = sb.dir.join("link.txt");
    std::os::unix::fs::symlink(&real, &link).unwrap();
    let p = link.to_str().unwrap();

    // The human line names both the path given and the file written.
    let out = run(&["replace", p, "--find", "hello", "--with", "hey"]);
    assert_eq!(code(&out), 0, "{}", String::from_utf8_lossy(&out.stderr));
    assert!(
        stdout(&out).contains(&format!("{p} -> {}", real.display())),
        "{}",
        stdout(&out)
    );

    // JSON carries the target as resolved_path, alongside the path as typed.
    let out = run(&["--json", "replace", p, "--find", "hey", "--with", "hello"]);
    let v: serde_json::Value = serde_json::from_str(&stdout(&out)).unwrap();
    assert_eq!(v["path"], p);
    assert_eq!(v["resolved_path"], real.display().to_string());

    // info reports it too.
    let out = run(&["--json", "info", p]);
    let v: serde_json::Value = serde_json::from_str(&stdout(&out)).unwrap();
    assert_eq!(v["resolved_path"], real.display().to_string());
    assert!(stdout(&run(&["info", p])).contains("symlink to:"));

    // An ordinary file gains neither the arrow nor the field.
    let q = real.to_str().unwrap();
    let out = run(&["replace", q, "--find", "hello", "--with", "hey"]);
    assert!(!stdout(&out).contains(" -> "), "{}", stdout(&out));
    let out = run(&["--json", "info", q]);
    let v: serde_json::Value = serde_json::from_str(&stdout(&out)).unwrap();
    assert!(v.get("resolved_path").is_none());
}

#[cfg(unix)]
#[test]
fn a_symlink_loop_is_refused() {
    let sb = Sandbox::new("symloop");
    std::os::unix::fs::symlink("b.txt", sb.dir.join("a.txt")).unwrap();
    std::os::unix::fs::symlink("a.txt", sb.dir.join("b.txt")).unwrap();

    // The loop is refused (the read hits ELOOP first), and neither link is
    // replaced by a regular file behind the user's back.
    let a = sb.dir.join("a.txt");
    let out = run(&["create", a.to_str().unwrap(), "--text", "hello"]);
    assert_eq!(code(&out), 9, "{}", String::from_utf8_lossy(&out.stderr));
    assert!(
        std::fs::symlink_metadata(&a)
            .unwrap()
            .file_type()
            .is_symlink()
    );
}

#[test]
fn search_reports_positions_and_exit_code() {
    let sb = Sandbox::new("search");
    let f = sb.file("a.txt", LATIN1);
    let p = f.to_str().unwrap();

    let out = run(&["--json", "search", p, "--find", "é"]);
    assert_eq!(code(&out), 0);
    let v: serde_json::Value = serde_json::from_str(&stdout(&out)).unwrap();
    assert_eq!(v["count"], 3);
    assert_eq!(v["matches"][0]["line"], 1);
    assert_eq!(v["matches"][0]["column"], 4);

    assert_eq!(code(&run(&["search", p, "--find", "zzz"])), 3);
    assert_eq!(
        code(&run(&["search", p, "--find", "zzz", "--allow-empty"])),
        0
    );
}

#[test]
fn regex_replace_with_capture_groups() {
    let sb = Sandbox::new("regex");
    let f = sb.file("a.py", b"def foo(a, b):\ndef bar(c):\n");
    let p = f.to_str().unwrap();

    let out = run(&[
        "replace",
        p,
        "--regex",
        "--all",
        "--find",
        r"def (\w+)\(",
        "--with",
        "def test_$1(",
    ]);
    assert_eq!(code(&out), 0, "{}", String::from_utf8_lossy(&out.stderr));
    assert_eq!(
        read(&f),
        b"def test_foo(a, b):\ndef test_bar(c):\n".to_vec()
    );
}

#[test]
fn replace_restricted_to_a_line_range() {
    let sb = Sandbox::new("region");
    let f = sb.file("a.txt", b"x\nx\nx\n");
    let p = f.to_str().unwrap();
    let out = run(&["replace", p, "--find", "x", "--with", "y", "--lines", "2"]);
    assert_eq!(code(&out), 0, "{}", String::from_utf8_lossy(&out.stderr));
    assert_eq!(read(&f), b"x\ny\nx\n".to_vec());
}

#[test]
fn view_prints_requested_lines() {
    let sb = Sandbox::new("view");
    let f = sb.file("a.txt", LATIN1);
    let out = run(&["view", f.to_str().unwrap(), "--lines", "2", "--number"]);
    assert_eq!(code(&out), 0);
    assert_eq!(stdout(&out), "     2\trésumé\n");
}

#[test]
fn create_refuses_to_clobber() {
    let sb = Sandbox::new("create");
    let f = sb.file("a.txt", b"existing\n");
    assert_eq!(
        code(&run(&["create", f.to_str().unwrap(), "--text", "new"])),
        7
    );

    let g = sb.dir.join("new.txt");
    let out = run(&[
        "--encoding",
        "windows-1252",
        "create",
        g.to_str().unwrap(),
        "--text",
        "café",
    ]);
    assert_eq!(code(&out), 0, "{}", String::from_utf8_lossy(&out.stderr));
    assert_eq!(read(&g), b"caf\xE9\n".to_vec());
}

#[test]
fn batch_applies_operations_atomically() {
    let sb = Sandbox::new("batch");
    let f = sb.file("a.txt", LATIN1);
    let script = sb.file(
        "script.json",
        r#"{"ops":[
              {"op":"replace","find":"café","with":"thé"},
              {"op":"append","text":"à demain"},
              {"op":"insert","line":1,"text":"début"}
            ]}"#
        .as_bytes(),
    );

    let out = run(&[
        "batch",
        f.to_str().unwrap(),
        "--script",
        script.to_str().unwrap(),
    ]);
    assert_eq!(code(&out), 0, "{}", String::from_utf8_lossy(&out.stderr));
    assert_eq!(
        read(&f),
        b"d\xE9but\nth\xE9\nr\xE9sum\xE9\n\xE0 demain\n".to_vec()
    );
}

#[test]
fn batch_moves_lines_against_the_running_state() {
    let sb = Sandbox::new("batchmove");
    let f = sb.file("a.txt", b"1\n2\n3\n4\n5\n");
    let script = sb.file(
        "script.json",
        r#"[{"op":"move-lines","lines":"1:2","after":"$"},
            {"op":"move-lines","lines":"$","by":-2},
            {"op":"delete","lines":1}]"#
            .as_bytes(),
    );

    let out = run(&[
        "batch",
        f.to_str().unwrap(),
        "--script",
        script.to_str().unwrap(),
    ]);
    assert_eq!(code(&out), 0, "{}", String::from_utf8_lossy(&out.stderr));
    // 1 2 3 4 5 -> 3 4 5 1 2 -> 3 4 2 5 1 -> 4 2 5 1
    assert_eq!(read(&f), b"4\n2\n5\n1\n".to_vec());
}

#[test]
fn batch_failure_leaves_the_file_untouched() {
    let sb = Sandbox::new("batchfail");
    let f = sb.file("a.txt", LATIN1);
    let script = sb.file(
        "script.json",
        r#"[{"op":"replace","find":"café","with":"thé"},
            {"op":"replace","find":"missing","with":"x"}]"#
            .as_bytes(),
    );
    let out = run(&[
        "batch",
        f.to_str().unwrap(),
        "--script",
        script.to_str().unwrap(),
    ]);
    assert_eq!(code(&out), 3);
    assert_eq!(read(&f), LATIN1.to_vec());
}

/// batch is the only multi-file mode. Each file keeps its own encoding, which
/// is the whole reason every other command takes exactly one.
#[test]
fn batch_edits_several_files_each_in_its_own_encoding() {
    let sb = Sandbox::new("batchmulti");
    let utf8 = sb.file("a.txt", "café\nold\n".as_bytes());
    let latin1 = sb.file("b.txt", b"caf\xE9\nold\n");

    let script = sb.file(
        "ops.json",
        format!(
            r#"[{{"op":"replace","file":{a:?},"find":"old","with":"nouveauté"}},
                {{"op":"replace","file":{b:?},"find":"old","with":"nouveauté"}},
                {{"op":"append","file":{a:?},"text":"fin"}}]"#,
            a = utf8.to_str().unwrap(),
            b = latin1.to_str().unwrap(),
        )
        .as_bytes(),
    );

    let out = run(&["batch", "--script", script.to_str().unwrap()]);
    assert_eq!(code(&out), 0, "{}", String::from_utf8_lossy(&out.stderr));

    // The UTF-8 file gets UTF-8 bytes; the windows-1252 one gets 0xE9.
    assert_eq!(read(&utf8), "café\nnouveauté\nfin\n".as_bytes().to_vec());
    assert_eq!(read(&latin1), b"caf\xE9\nnouveaut\xE9\n".to_vec());

    // One summary line per file, in first-touched order.
    let text = stdout(&out);
    assert_eq!(text.lines().count(), 2, "{text}");
    assert!(text.contains("windows-1252"), "{text}");
}

/// A failure anywhere in the script leaves *every* file as it was, not just
/// the one the failing operation named.
#[test]
fn batch_failure_leaves_every_file_untouched() {
    let sb = Sandbox::new("batchmultifail");
    let a = sb.file("a.txt", b"one\nold\n");
    let b = sb.file("b.txt", b"two\nold\n");
    let script = sb.file(
        "ops.json",
        format!(
            r#"[{{"op":"replace","file":{a:?},"find":"old","with":"new"}},
                {{"op":"replace","file":{b:?},"find":"old","with":"new"}},
                {{"op":"replace","file":{b:?},"find":"absent","with":"x"}}]"#,
            a = a.to_str().unwrap(),
            b = b.to_str().unwrap(),
        )
        .as_bytes(),
    );

    let out = run(&["batch", "--script", script.to_str().unwrap()]);
    assert_eq!(code(&out), 3);
    // The error names both the operation index and the file it was aimed at.
    let err = String::from_utf8_lossy(&out.stderr);
    assert!(err.contains("operation 3"), "{err}");
    assert!(err.contains("b.txt"), "{err}");
    assert_eq!(read(&a), b"one\nold\n".to_vec(), "a.txt was written");
    assert_eq!(read(&b), b"two\nold\n".to_vec(), "b.txt was written");
}

#[test]
fn batch_file_argument_is_the_default_for_ops_without_one() {
    let sb = Sandbox::new("batchdefault");
    let a = sb.file("a.txt", b"one\nold\n");
    let b = sb.file("b.txt", b"two\n");
    let script = sb.file(
        "ops.json",
        format!(
            r#"[{{"op":"replace","find":"old","with":"new"}},
                {{"op":"append","file":{b:?},"text":"end"}}]"#,
            b = b.to_str().unwrap(),
        )
        .as_bytes(),
    );

    let out = run(&[
        "batch",
        a.to_str().unwrap(),
        "--script",
        script.to_str().unwrap(),
    ]);
    assert_eq!(code(&out), 0, "{}", String::from_utf8_lossy(&out.stderr));
    assert_eq!(read(&a), b"one\nnew\n".to_vec());
    assert_eq!(read(&b), b"two\nend\n".to_vec());

    // An op with no file and no default is a usage error naming the operation.
    let script = sb.file("bare.json", br#"[{"op":"append","text":"x"}]"#);
    let out = run(&["batch", "--script", script.to_str().unwrap()]);
    assert_eq!(code(&out), 2);
    assert!(String::from_utf8_lossy(&out.stderr).contains("operation 1"));
}

/// The result shape must not change with the number of files: a caller should
/// never have to branch on the count.
#[test]
fn batch_json_always_reports_a_files_array() {
    let sb = Sandbox::new("batchjson");
    let a = sb.file("a.txt", b"one\nold\n");
    let script = sb.file(
        "ops.json",
        br#"[{"op":"replace","find":"old","with":"new"}]"#,
    );

    let out = run(&[
        "--json",
        "batch",
        a.to_str().unwrap(),
        "--script",
        script.to_str().unwrap(),
    ]);
    assert_eq!(code(&out), 0, "{}", String::from_utf8_lossy(&out.stderr));
    let v: serde_json::Value = serde_json::from_str(&stdout(&out)).unwrap();
    assert_eq!(v["ok"], true);
    assert_eq!(v["command"], "batch");
    assert_eq!(v["operations"], 1);
    assert_eq!(v["changed"], true);
    let files = v["files"].as_array().expect("files array");
    assert_eq!(files.len(), 1);
    assert_eq!(files[0]["summary"], "applied 1 operation(s)");
    assert_eq!(files[0]["encoding"], "UTF-8");
    assert!(files[0]["path"].as_str().unwrap().ends_with("a.txt"));
}

#[test]
fn undecodable_file_is_refused_without_lossy() {
    let sb = Sandbox::new("broken");
    // A lone 0x80 is not valid UTF-8; force UTF-8 so detection cannot rescue it.
    let f = sb.file("a.txt", b"ok\x80\nline\n");
    let p = f.to_str().unwrap();

    let out = run(&[
        "--encoding",
        "utf-8",
        "replace",
        p,
        "--find",
        "line",
        "--with",
        "row",
    ]);
    assert_eq!(code(&out), 5);
    assert_eq!(read(&f), b"ok\x80\nline\n".to_vec());

    let out = run(&[
        "--encoding",
        "utf-8",
        "--lossy",
        "replace",
        p,
        "--find",
        "line",
        "--with",
        "row",
    ]);
    assert_eq!(code(&out), 0, "{}", String::from_utf8_lossy(&out.stderr));
}

#[test]
fn binary_files_are_refused_by_default() {
    let sb = Sandbox::new("binary");
    let f = sb.file("a.bin", b"abc\x00def\n");
    let out = run(&[
        "replace",
        f.to_str().unwrap(),
        "--find",
        "abc",
        "--with",
        "xyz",
    ]);
    assert_eq!(code(&out), 5);
}

/// The guard covers reading, not only writing: `view` on a binary would
/// otherwise emit raw NULs and escape sequences and exit 0.
#[test]
fn binary_files_are_refused_by_the_reading_commands_too() {
    let sb = Sandbox::new("binaryread");
    let f = sb.file("a.bin", b"abc\x00def\n");
    let p = f.to_str().unwrap();

    for args in [
        vec!["view", p],
        vec!["search", p, "--find", "abc"],
        vec!["convert", p, "--to", "utf-8"],
    ] {
        let out = run(&args);
        assert_eq!(code(&out), 5, "{} was not refused", args[0]);
        assert!(out.stdout.is_empty(), "{} wrote to stdout", args[0]);
    }

    // --force is the single override, for reads as for writes.
    let out = run(&["view", p, "--force"]);
    assert_eq!(code(&out), 0);
    assert_eq!(out.stdout, b"abc\x00def\n");
}

/// `info` is the exception: it is how a caller learns why the rest refused,
/// so it always reports. What it reports for a non-text file is the byte-level
/// truth and the verdict, and nothing derived from decoding it - an encoding
/// guessed for a blob, and the line endings of the result, are not facts about
/// the file.
#[test]
fn info_reports_the_verdict_and_withholds_the_text_report() {
    let sb = Sandbox::new("binaryinfo");
    let f = sb.file("a.bin", b"abc\x00def\n");
    let p = f.to_str().unwrap();

    let out = run(&["info", p]);
    assert_eq!(code(&out), 0);
    let text = stdout(&out);
    assert!(text.contains("not text:"), "{text}");
    assert!(text.contains("NUL byte at offset 3"), "{text}");
    for withheld in [
        "encoding:",
        "bom:",
        "line endings:",
        "lines:",
        "characters:",
        "final newline:",
        "edit safety:",
    ] {
        assert!(!text.contains(withheld), "{withheld} survived:\n{text}");
    }
    // What is true of the bytes stays.
    assert!(text.contains("bytes:           8"), "{text}");

    let v: serde_json::Value = serde_json::from_str(&stdout(&run(&["info", p, "--json"]))).unwrap();
    assert_eq!(v["looks_binary"], true);
    assert_eq!(v["binary"]["reason"], "nul");
    assert_eq!(v["binary"]["offset"], 3);
    assert_eq!(v["bytes"], 8);
    for withheld in [
        "encoding",
        "detected_by",
        "bom",
        "eol",
        "lines",
        "characters",
    ] {
        assert!(v.get(withheld).is_none(), "{withheld} survived: {v}");
    }
}

/// --force means "treat this as text" for `info` as it does everywhere else,
/// so the withheld report comes back in full - with the verdict still on top.
#[test]
fn info_force_prints_the_text_report_for_a_binary() {
    let sb = Sandbox::new("binaryinfoforce");
    let f = sb.file("a.bin", b"abc\x00def\n");
    let p = f.to_str().unwrap();

    let text = stdout(&run(&["info", p, "--force"]));
    assert!(text.contains("not text:"), "{text}");
    assert!(text.contains("encoding:"), "{text}");
    assert!(text.contains("line endings:"), "{text}");
    assert!(text.contains("edit safety:"), "{text}");

    let v: serde_json::Value =
        serde_json::from_str(&stdout(&run(&["info", p, "--json", "--force"]))).unwrap();
    assert_eq!(v["looks_binary"], true);
    assert!(v.get("encoding").is_some(), "{v}");
    assert!(v.get("binary").is_some(), "{v}");
}

/// The mojibake shape is two ordinary bytes in sequence, so any blob turns it
/// up by chance. Reporting it alongside "this is not a text file" would tell
/// the reader to go and report damage in an ELF binary.
#[test]
fn binary_files_are_not_also_reported_as_mojibake() {
    let sb = Sandbox::new("binarymoji");
    // "Ã©" in windows-1252 is the canonical mojibake shape; the NUL is what
    // makes this a non-text file.
    let f = sb.file("a.bin", b"\x00\xC3\xA9 \xC3\xA9 \xC3\xA9 \xC3\xA9\n");
    let p = f.to_str().unwrap();

    let out = run(&["info", p]);
    let text = stdout(&out);
    assert!(!text.contains("mojibake"), "{text}");
    let v: serde_json::Value = serde_json::from_str(&stdout(&run(&["info", p, "--json"]))).unwrap();
    assert!(v.get("mojibake").is_none(), "{v}");

    // Nor on the write path, where --force has got past the guard.
    let out = run(&["append", p, "--text", "x", "--force"]);
    assert_eq!(code(&out), 0, "{}", String::from_utf8_lossy(&out.stderr));
    assert!(
        !String::from_utf8_lossy(&out.stderr).contains("mojibake"),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );

    // A text file carrying the same shape still gets the warning. Written as
    // double-encoded UTF-8, since without the NUL the file decodes as UTF-8
    // and the windows-1252 reading that turns C3 A9 into "Ã©" never happens.
    let g = sb.file(
        "b.txt",
        b"caf\xC3\x83\xC2\xA9 r\xC3\x83\xC2\xA9sum\xC3\x83\xC2\xA9\n",
    );
    assert!(stdout(&run(&["info", g.to_str().unwrap()])).contains("mojibake"));
}

/// An ordinary text file is unaffected by any of the above: it still gets the
/// full report, dominant line ending and all.
#[test]
fn text_files_keep_the_whole_report() {
    let sb = Sandbox::new("binaryeol");
    let g = sb.file("b.txt", b"abcd\ne\nf\n");
    let text = stdout(&run(&["info", g.to_str().unwrap()]));
    assert!(text.contains("line endings:    lf ("), "{text}");
    assert!(text.contains("edit safety:     byte-exact"), "{text}");
    assert!(!text.contains("not text:"), "{text}");
}

/// High-entropy data with no NUL in it: the case the old NUL-only check let
/// through.
#[test]
fn binary_without_nul_bytes_is_refused() {
    let sb = Sandbox::new("binarynonul");
    let mut data = Vec::new();
    let mut state: u64 = 0x2545_F491_4F6C_DD1D;
    while data.len() < 4000 {
        state = state
            .wrapping_mul(6364136223846793005)
            .wrapping_add(1442695040888963407);
        let b = (state >> 33) as u8;
        if b != 0 {
            data.push(b);
        }
    }
    let f = sb.file("a.bin", &data);
    let out = run(&["view", f.to_str().unwrap()]);
    assert_eq!(code(&out), 5);
    assert!(
        String::from_utf8_lossy(&out.stderr).contains("control characters"),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
}

/// UTF-16 with no BOM is text nothing declared the encoding of, not binary.
/// The refusal has to name the flag that fixes it, or it is a dead end:
/// detection cannot find UTF-16 unaided.
#[test]
fn bom_less_utf16_is_refused_with_the_encoding_that_reads_it() {
    let sb = Sandbox::new("utf16nobom");
    let bytes: Vec<u8> = "héllo wörld\nsecond line\n"
        .encode_utf16()
        .flat_map(|u| u.to_le_bytes())
        .collect();
    let f = sb.file("a.txt", &bytes);
    let p = f.to_str().unwrap();

    let out = run(&["view", p]);
    assert_eq!(code(&out), 5);
    let err = String::from_utf8_lossy(&out.stderr).into_owned();
    assert!(err.contains("UTF-16LE with no BOM"), "{err}");
    assert!(err.contains("--encoding utf-16le"), "{err}");

    // And that flag really does read it.
    let out = run(&["view", p, "--encoding", "utf-16le"]);
    assert_eq!(code(&out), 0, "{}", String::from_utf8_lossy(&out.stderr));
    assert_eq!(stdout(&out), "héllo wörld\nsecond line\n");
}

/// A BOM'd UTF-16 file is ordinary text and must stay unaffected by all of
/// the above - its bytes are half NULs.
#[test]
fn utf16_with_a_bom_is_not_treated_as_binary() {
    let sb = Sandbox::new("utf16bom");
    let mut bytes = vec![0xFF, 0xFE];
    bytes.extend("héllo\n".encode_utf16().flat_map(|u| u.to_le_bytes()));
    let f = sb.file("a.txt", &bytes);
    let out = run(&["view", f.to_str().unwrap()]);
    assert_eq!(code(&out), 0, "{}", String::from_utf8_lossy(&out.stderr));
    assert_eq!(stdout(&out), "héllo\n");
}

#[test]
fn missing_file_exits_eight() {
    let sb = Sandbox::new("missing");
    let f = sb.dir.join("nope.txt");
    assert_eq!(code(&run(&["view", f.to_str().unwrap()])), 8);
}

#[test]
fn creating_an_empty_file_actually_creates_it() {
    let sb = Sandbox::new("emptycreate");
    let f = sb.dir.join("empty.txt");
    assert_eq!(
        code(&run(&["create", f.to_str().unwrap(), "--text", ""])),
        0
    );
    assert!(f.exists(), "create with empty text did not create the file");
    assert_eq!(read(&f), Vec::<u8>::new());

    let g = sb.dir.join("blank.txt");
    assert_eq!(code(&run(&["write", g.to_str().unwrap(), "--text", ""])), 0);
    assert!(g.exists(), "write with empty text did not create the file");
}

#[test]
fn missing_parent_directory_is_explained_and_creatable() {
    let sb = Sandbox::new("parents");
    let f = sb.dir.join("src/components/Foo.tsx");
    let p = f.to_str().unwrap();

    let out = run(&["create", p, "--text", "x"]);
    assert_eq!(code(&out), 8);
    let err = String::from_utf8_lossy(&out.stderr);
    assert!(err.contains("does not exist"), "unhelpful error: {err}");
    assert!(
        err.contains("--parents"),
        "error does not mention the fix: {err}"
    );

    assert_eq!(code(&run(&["create", p, "--parents", "--text", "x"])), 0);
    assert_eq!(read(&f), b"x\n".to_vec());

    let g = sb.dir.join("a/b/c.txt");
    assert_eq!(
        code(&run(&["write", g.to_str().unwrap(), "-p", "--text", "hi"])),
        0
    );
    assert_eq!(read(&g), b"hi\n".to_vec());
}

fn run_env(env: &[(&str, &str)], args: &[&str]) -> Output {
    let mut cmd = Command::new(EXE);
    cmd.args(args);
    for (k, v) in env {
        cmd.env(k, v);
    }
    cmd.output().expect("failed to run intact")
}

/// A project that mandates one encoding states it with --encoding on every
/// command, including `create`, which otherwise makes UTF-8 files.
#[test]
fn the_encoding_flag_covers_creation_and_editing() {
    let sb = Sandbox::new("encflag");

    // A new file is created in the mandated encoding, not UTF-8.
    let f = sb.dir.join("notes.txt");
    let p = f.to_str().unwrap();
    assert_eq!(
        code(&run(&[
            "--encoding",
            "latin1",
            "create",
            p,
            "--text",
            "Olá mundo"
        ])),
        0
    );
    assert_eq!(read(&f), b"Ol\xE1 mundo\n".to_vec());

    // Edits use it too, with no detection in play. This exact edit fails when
    // the encoding is guessed, because a short file can be read as
    // windows-1250, which has no 'ã'.
    let out = run(&[
        "--encoding",
        "latin1",
        "replace",
        p,
        "--find",
        "mundo",
        "--with",
        "mundão",
    ]);
    assert_eq!(code(&out), 0, "{}", String::from_utf8_lossy(&out.stderr));
    assert_eq!(read(&f), b"Ol\xE1 mund\xE3o\n".to_vec());

    // info attributes the encoding to the flag.
    let out = run(&["--json", "--encoding", "latin1", "info", p]);
    let v: serde_json::Value = serde_json::from_str(&stdout(&out)).unwrap();
    assert_eq!(v["encoding"], "windows-1252");
    assert_eq!(v["detected_by"], "explicit");

    // A bad label is a usage error naming the flag's value.
    let out = run(&["--encoding", "nonsense-9", "info", p]);
    assert_eq!(code(&out), 2);
}

#[test]
fn no_guess_refuses_writes_to_undeclared_encodings() {
    let sb = Sandbox::new("noguess");
    let f = sb.file("a.txt", LATIN1);
    let p = f.to_str().unwrap();

    // Guessed encoding + strict mode = refusal, by flag or by environment.
    let out = run(&[
        "--no-guess",
        "replace",
        p,
        "--find",
        "café",
        "--with",
        "thé",
    ]);
    assert_eq!(code(&out), 5);
    assert!(String::from_utf8_lossy(&out.stderr).contains("guessed"));
    assert_eq!(read(&f), LATIN1.to_vec());

    // Declaring the encoding satisfies it.
    let out = run(&[
        "--no-guess",
        "--encoding",
        "latin1",
        "replace",
        p,
        "--find",
        "café",
        "--with",
        "thé",
    ]);
    assert_eq!(code(&out), 0, "{}", String::from_utf8_lossy(&out.stderr));

    // Read-only commands still work, so a tripped guard can be diagnosed.
    assert_eq!(code(&run(&["--no-guess", "info", p])), 0);
    assert_eq!(code(&run(&["--no-guess", "view", p])), 0);

    // A UTF-8 file is not a guess, so strict mode does not touch it.
    let g = sb.file("b.txt", "héllo\n".as_bytes());
    assert_eq!(
        code(&run(&[
            "--no-guess",
            "replace",
            g.to_str().unwrap(),
            "--find",
            "héllo",
            "--with",
            "hi"
        ])),
        0
    );
}

/// A file with no non-ASCII byte reads identically under every ASCII superset,
/// so nothing in it says which one the project means it to be. `info` says as
/// much rather than claiming a UTF-8 detection it did not make.
#[test]
fn a_pure_ascii_file_reports_that_nothing_was_detected() {
    let sb = Sandbox::new("asciidetect");
    let f = sb.file("a.txt", b"plain text\n");
    let p = f.to_str().unwrap();

    let v: serde_json::Value = serde_json::from_str(&stdout(&run(&["info", p, "--json"]))).unwrap();
    assert_eq!(v["encoding"], "UTF-8");
    assert_eq!(v["detected_by"], "ascii");

    // The human report has to name the encoding a write would use, but must not
    // call it a detection: being ASCII is the evidence that nothing was
    // detected. ("US-ASCII" cannot stand in for the name either — that is what
    // `--encoding ascii` mandates, and nothing here mandated it.)
    let text = stdout(&run(&["info", p]));
    assert!(
        text.contains("encoding:        UTF-8 (assumed - every byte is ASCII)"),
        "{text}"
    );
    assert!(!text.contains("detected by: ascii"), "{text}");

    // One non-ASCII byte is a real detection, and reported as one.
    let g = sb.file("b.txt", "héllo\n".as_bytes());
    let v: serde_json::Value =
        serde_json::from_str(&stdout(&run(&["info", g.to_str().unwrap(), "--json"]))).unwrap();
    assert_eq!(v["detected_by"], "utf-8-valid");
}

/// The write that turns an undeclared ASCII file into a file of some definite
/// encoding: the only moment the missing --encoding changes the bytes on disk.
#[test]
fn writing_non_ascii_into_an_undeclared_ascii_file() {
    let sb = Sandbox::new("asciiguard");
    let f = sb.file("a.txt", b"cafe\n");
    let p = f.to_str().unwrap();

    // Under the mandate flag it is a refusal, and nothing is written.
    let out = run(&[
        "--no-guess",
        "replace",
        p,
        "--find",
        "cafe",
        "--with",
        "café",
    ]);
    assert_eq!(code(&out), 5);
    let err = String::from_utf8_lossy(&out.stderr);
    assert!(err.contains("never observed"), "stderr was: {err}");
    assert!(err.contains("--encoding"), "stderr was: {err}");
    assert_eq!(read(&f), b"cafe\n".to_vec());

    // An ASCII-only edit to the same file is unaffected: every encoding it
    // could be agrees about those bytes.
    assert_eq!(
        code(&run(&[
            "--no-guess",
            "replace",
            p,
            "--find",
            "cafe",
            "--with",
            "tea"
        ])),
        0
    );
    assert_eq!(read(&f), b"tea\n".to_vec());

    // Declaring the encoding is what the refusal asked for, and settles it.
    let g = sb.file("b.txt", b"cafe\n");
    let out = run(&[
        "--encoding",
        "windows-1252",
        "--no-guess",
        "replace",
        g.to_str().unwrap(),
        "--find",
        "cafe",
        "--with",
        "café",
    ]);
    assert_eq!(code(&out), 0, "{}", String::from_utf8_lossy(&out.stderr));
    assert_eq!(read(&g), b"caf\xE9\n".to_vec());

    // Without --no-guess the write proceeds — but says what it decided.
    let h = sb.file("c.txt", b"cafe\n");
    let out = run(&[
        "replace",
        h.to_str().unwrap(),
        "--find",
        "cafe",
        "--with",
        "café",
    ]);
    assert_eq!(code(&out), 0);
    let err = String::from_utf8_lossy(&out.stderr);
    assert!(err.contains("warning"), "stderr was: {err}");
    assert!(err.contains("UTF-8 from here on"), "stderr was: {err}");
    assert_eq!(read(&h), "café\n".as_bytes().to_vec());
}

/// `--encoding ascii` is the standing version of that guard: it says the file
/// must stay ASCII, so the character is refused however it was arrived at,
/// rather than the flag quietly meaning windows-1252 as the WHATWG label does.
#[test]
fn declaring_ascii_refuses_every_non_ascii_write() {
    let sb = Sandbox::new("asciimandate");
    let f = sb.file("a.txt", b"cafe\n");
    let p = f.to_str().unwrap();

    let out = run(&[
        "-e", "ascii", "replace", p, "--find", "cafe", "--with", "café",
    ]);
    assert_eq!(code(&out), 5);
    let err = String::from_utf8_lossy(&out.stderr);
    assert!(err.contains("US-ASCII"), "stderr was: {err}");
    assert!(err.contains("U+00E9"), "stderr was: {err}");
    assert_eq!(read(&f), b"cafe\n".to_vec());

    // The label is a declaration, so the file reports as declared, not guessed.
    let v: serde_json::Value =
        serde_json::from_str(&stdout(&run(&["-e", "ascii", "info", p, "--json"]))).unwrap();
    assert_eq!(v["encoding"], "US-ASCII");
    assert_eq!(v["detected_by"], "explicit");

    // Its aliases mean the same thing, and ASCII-only edits still go through.
    for label in ["us-ascii", "ANSI_X3.4-1968", "iso646-us"] {
        let out = run(&[
            "-e", label, "replace", p, "--find", "cafe", "--with", "café",
        ]);
        assert_eq!(code(&out), 5, "label {label}");
    }
    assert_eq!(
        code(&run(&[
            "-e", "ascii", "replace", p, "--find", "cafe", "--with", "tea"
        ])),
        0
    );
    assert_eq!(read(&f), b"tea\n".to_vec());

    // Naming an encoding that has the character is what gets it in - the whole
    // point of the refusal being that the caller chooses which encoding that is.
    let out = run(&[
        "-e",
        "windows-1252",
        "replace",
        p,
        "--find",
        "tea",
        "--with",
        "café",
    ]);
    assert_eq!(code(&out), 0, "{}", String::from_utf8_lossy(&out.stderr));
    assert_eq!(read(&f), b"caf\xE9\n".to_vec());

    // And that file is no longer editable as ASCII at all.
    let out = run(&[
        "-e", "ascii", "replace", p, "--find", "caf", "--with", "tea",
    ]);
    assert_eq!(code(&out), 5);
    let err = String::from_utf8_lossy(&out.stderr);
    assert!(err.contains("not ASCII"), "stderr was: {err}");
}

/// `convert --to ascii` is the same rule applied to a whole file: a check that
/// it is ASCII, or - with --unmappable - a way to make it so.
#[test]
fn converting_to_ascii_reports_what_does_not_fit() {
    let sb = Sandbox::new("asciiconvert");
    let f = sb.file("a.txt", "café\n".as_bytes());
    let p = f.to_str().unwrap();

    let out = run(&["convert", p, "--to", "ascii"]);
    assert_eq!(code(&out), 5);
    assert_eq!(read(&f), "café\n".as_bytes().to_vec());

    // ASCII has no byte-order mark, so asking for one is a usage error rather
    // than a silently dropped flag.
    assert_eq!(
        code(&run(&["convert", p, "--to", "ascii", "--bom", "add"])),
        2
    );

    let out = run(&["convert", p, "--to", "ascii", "--unmappable", "xml"]);
    assert_eq!(code(&out), 0, "{}", String::from_utf8_lossy(&out.stderr));
    assert_eq!(read(&f), b"caf&#233;\n".to_vec());
}

/// A batch is a transaction, so the guard has to stop it before any of its
/// files are written, not after the operation that trips it.
#[test]
fn the_ascii_guard_covers_batch() {
    let sb = Sandbox::new("asciibatch");
    let a = sb.file("a.txt", b"one\n");
    let b = sb.file("b.txt", b"two\n");
    let script = sb.file(
        "ops.json",
        format!(
            r#"[{{"op":"replace","file":{:?},"find":"one","with":"uno"}},
                {{"op":"replace","file":{:?},"find":"two","with":"deux é"}}]"#,
            a.display().to_string(),
            b.display().to_string()
        )
        .as_bytes(),
    );

    let out = run(&["--no-guess", "batch", "--script", script.to_str().unwrap()]);
    assert_eq!(code(&out), 5);
    // The first operation succeeded and is still discarded.
    assert_eq!(read(&a), b"one\n".to_vec());
    assert_eq!(read(&b), b"two\n".to_vec());

    // Without the flag it applies, with one warning naming the file it settled.
    let out = run(&["batch", "--script", script.to_str().unwrap()]);
    assert_eq!(code(&out), 0, "{}", String::from_utf8_lossy(&out.stderr));
    let err = String::from_utf8_lossy(&out.stderr);
    assert_eq!(err.matches("warning").count(), 1, "stderr was: {err}");
    assert!(err.contains("b.txt"), "stderr was: {err}");
    assert_eq!(read(&a), b"uno\n".to_vec());
}

/// A regex `\n` matches a bare LF, so a pattern spanning lines finds nothing in
/// a CRLF file. The literal path is shaped to the file's terminators and does
/// not have the problem, which is exactly what makes the regex one surprising.
#[test]
fn a_regex_spanning_lines_explains_itself_on_a_crlf_file() {
    let sb = Sandbox::new("crlfregex");
    let f = sb.file("a.txt", b"alpha\r\nbeta\r\n");
    let p = f.to_str().unwrap();

    let out = run(&["search", p, "--regex", "--find", r"alpha\nbeta"]);
    assert_eq!(code(&out), 3);
    let err = String::from_utf8_lossy(&out.stderr);
    assert!(err.contains(r"\r?\n"), "stderr was: {err}");

    // The same on the write side, as the hint of the no-match error.
    let out = run(&[
        "replace",
        p,
        "--regex",
        "--find",
        r"alpha\nbeta",
        "--with",
        "x",
    ]);
    assert_eq!(code(&out), 3);
    let err = String::from_utf8_lossy(&out.stderr);
    assert!(err.contains(r"\r?\n"), "stderr was: {err}");

    // And in JSON, where a caller reads it as a field rather than off stderr.
    let out = run(&["--json", "search", p, "--regex", "--find", r"alpha\nbeta"]);
    let v: serde_json::Value = serde_json::from_str(&stdout(&out)).unwrap();
    assert_eq!(v["count"], 0);
    assert!(v["hint"].as_str().unwrap().contains(r"\r?\n"), "{v}");

    // Taking the advice works.
    let out = run(&["search", p, "--regex", "--find", r"alpha\r?\nbeta"]);
    assert_eq!(code(&out), 0, "{}", String::from_utf8_lossy(&out.stderr));

    // The hint is specific to this failure: it stays off when the pattern has
    // already accounted for CR, when the file has no CRLF, and when a miss has
    // nothing to do with line endings.
    for args in [
        vec!["search", p, "--regex", "--find", r"alpha\r?\nzzz"],
        vec!["search", p, "--regex", "--find", r"zzz"],
    ] {
        let out = run(&args);
        assert_eq!(code(&out), 3);
        let err = String::from_utf8_lossy(&out.stderr);
        assert!(!err.contains("CRLF"), "{args:?} hinted: {err}");
    }
    let g = sb.file("b.txt", b"alpha\nbeta\n");
    let out = run(&[
        "search",
        g.to_str().unwrap(),
        "--regex",
        "--find",
        r"a\nzzz",
    ]);
    assert_eq!(code(&out), 3);
    assert!(!String::from_utf8_lossy(&out.stderr).contains("CRLF"));
}

/// The line-ending equivalent of the encoding mandate.
#[test]
fn the_eol_flag_covers_creation_and_editing() {
    let sb = Sandbox::new("eolflag");

    // A new file gets the requested endings, not the LF default.
    let f = sb.dir.join("new.txt");
    let p = f.to_str().unwrap();
    assert_eq!(
        code(&run(&[
            "--eol",
            "crlf",
            "create",
            p,
            "--escapes",
            "--text",
            "a\\nb"
        ])),
        0
    );
    assert_eq!(read(&f), b"a\r\nb\r\n".to_vec());

    // Inserted text too.
    assert_eq!(
        code(&run(&["--eol", "crlf", "append", p, "--text", "c"])),
        0
    );
    assert_eq!(read(&f), b"a\r\nb\r\nc\r\n".to_vec());

    // Without the flag, a new file follows the LF default.
    let g = sb.dir.join("lf.txt");
    assert_eq!(
        code(&run(&[
            "create",
            g.to_str().unwrap(),
            "--escapes",
            "--text",
            "a\\nb"
        ])),
        0
    );
    assert_eq!(read(&g), b"a\nb\n".to_vec());

    // A bad value is a usage error.
    assert_eq!(code(&run(&["--eol", "wobbly", "view", p])), 2);
}

#[test]
fn strict_eol_refuses_to_create_mixed_endings() {
    let sb = Sandbox::new("strricteol");
    let f = sb.file("lf.txt", b"one\ntwo\n");
    let p = f.to_str().unwrap();

    // Appending CRLF text to an LF file would leave it mixed: refuse instead.
    let out = run(&[
        "--eol",
        "crlf",
        "--strict-eol",
        "append",
        p,
        "--text",
        "three",
    ]);
    assert_eq!(code(&out), 5);
    let err = String::from_utf8_lossy(&out.stderr);
    assert!(err.contains("not CRLF"), "{err}");
    assert!(err.contains("--newlines crlf"), "hint missing: {err}");
    assert_eq!(read(&f), b"one\ntwo\n".to_vec());

    // The suggested remedy needs no --to, and then the edit succeeds.
    assert_eq!(code(&run(&["convert", p, "--newlines", "crlf"])), 0);
    assert_eq!(read(&f), b"one\r\ntwo\r\n".to_vec());
    assert_eq!(
        code(&run(&[
            "--eol",
            "crlf",
            "--strict-eol",
            "append",
            p,
            "--text",
            "three"
        ])),
        0
    );
    assert_eq!(read(&f), b"one\r\ntwo\r\nthree\r\n".to_vec());

    // A conforming file is untouched by the guard, and write/create are exempt
    // because they replace the whole content anyway.
    let g = sb.file("mixed.txt", b"a\r\nb\n");
    assert_eq!(
        code(&run(&[
            "--eol",
            "crlf",
            "--strict-eol",
            "write",
            g.to_str().unwrap(),
            "--text",
            "x"
        ])),
        0
    );

    // The guard needs a concrete style to enforce.
    let out = run(&["--strict-eol", "append", p, "--text", "z"]);
    assert_eq!(code(&out), 2);
    assert!(String::from_utf8_lossy(&out.stderr).contains("--eol"));
}

#[test]
fn convert_can_normalise_line_endings_alone() {
    let sb = Sandbox::new("convertnl");
    // A Latin-1 file with mixed endings: fix the endings, keep the encoding.
    let f = sb.file("a.txt", b"caf\xE9\r\nth\xE9\nfin\r\n");
    let p = f.to_str().unwrap();

    let out = run(&["--encoding", "latin1", "convert", p, "--newlines", "lf"]);
    assert_eq!(code(&out), 0, "{}", String::from_utf8_lossy(&out.stderr));
    assert_eq!(read(&f), b"caf\xE9\nth\xE9\nfin\n".to_vec());

    // `auto` collapses mixed endings onto the file's own dominant style.
    let g = sb.file("b.txt", b"a\r\nb\r\nc\n");
    assert_eq!(
        code(&run(&[
            "convert",
            g.to_str().unwrap(),
            "--newlines",
            "auto"
        ])),
        0
    );
    assert_eq!(read(&g), b"a\r\nb\r\nc\r\n".to_vec());

    // Neither --to nor --newlines is a usage error.
    assert_eq!(code(&run(&["convert", p])), 2);
}

#[test]
fn instructions_can_state_a_project_encoding_mandate() {
    let text = stdout(&run(&["instructions", "--encoding", "latin1"]));
    assert!(text.contains("Encoding: always `windows-1252`"));
    assert!(text.contains("--encoding windows-1252 --no-guess"));

    let brief = stdout(&run(&[
        "instructions",
        "--brief",
        "--encoding",
        "shift_jis",
    ]));
    assert!(brief.contains("Shift_JIS"));

    // Without the flag, no encoding policy is asserted.
    let plain = stdout(&run(&["instructions"]));
    assert!(!plain.contains("Encoding: always"));
}

/// The generated section must never send an agent to an environment variable:
/// each of its commands may run in a fresh shell, so an `export` would not
/// survive to the next one.
#[test]
fn instructions_never_mention_environment_variables() {
    for args in [
        vec!["instructions"],
        vec!["instructions", "--brief"],
        vec!["instructions", "--encoding", "latin1", "--eol", "crlf"],
        vec![
            "instructions",
            "--brief",
            "--encoding",
            "latin1",
            "--eol",
            "crlf",
        ],
    ] {
        let text = stdout(&run(&args));
        assert!(!text.contains("INTACT_"), "{args:?} still names a variable");
        assert!(!text.contains("export "), "{args:?} still says `export`");
    }
}

/// batch is the answer to "this file needs several edits", so an agent handed
/// only the generated section has to learn it exists.
#[test]
fn instructions_recommend_batch_for_multiple_edits() {
    for args in [vec!["instructions"], vec!["instructions", "--brief"]] {
        let text = stdout(&run(&args));
        assert!(text.contains("batch"), "{args:?} does not mention batch");
        assert!(
            text.contains("\"file\""),
            "{args:?} does not show the multi-file form"
        );
    }
}

#[test]
fn instructions_can_state_a_line_ending_mandate() {
    let text = stdout(&run(&["instructions", "--eol", "crlf"]));
    assert!(text.contains("Line endings: always CRLF"));
    assert!(text.contains("--eol crlf --strict-eol"));
    assert!(text.contains("--newlines crlf"));

    // Both mandates can appear together.
    let both = stdout(&run(&[
        "instructions",
        "--encoding",
        "latin1",
        "--eol",
        "lf",
    ]));
    assert!(both.contains("Encoding: always `windows-1252`"));
    assert!(both.contains("Line endings: always LF"));

    let brief = stdout(&run(&["instructions", "--brief", "--eol", "crlf"]));
    assert!(brief.contains("--eol crlf --strict-eol"));

    // `auto` is not a mandate, so nothing is asserted.
    let plain = stdout(&run(&["instructions", "--eol", "auto"]));
    assert!(!plain.contains("Line endings: always"));
}

/// One convention for "read this from standard input", rather than a parallel
/// --x-stdin flag beside every path argument.
#[test]
fn a_dash_path_reads_standard_input() {
    let sb = Sandbox::new("dashstdin");
    let f = sb.file("a.txt", b"one\ntwo\n");
    let p = f.to_str().unwrap();

    let feed = |args: &[&str], input: &str| {
        use std::io::Write;
        use std::process::Stdio;
        let mut child = Command::new(EXE)
            .args(args)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .unwrap();
        child
            .stdin
            .take()
            .unwrap()
            .write_all(input.as_bytes())
            .unwrap();
        child.wait_with_output().unwrap()
    };

    assert_eq!(code(&feed(&["append", p, "--text-file", "-"], "three")), 0);
    assert_eq!(read(&f), b"one\ntwo\nthree\n".to_vec());

    let out = feed(&["replace", p, "--find", "one", "--with-file", "-"], "1");
    assert_eq!(code(&out), 0, "{}", String::from_utf8_lossy(&out.stderr));
    assert_eq!(read(&f), b"1\ntwo\nthree\n".to_vec());

    // The script argument has always used this convention; it still does.
    let out = feed(
        &["batch", p, "--script", "-"],
        r#"[{"op":"replace","find":"two","with":"2"}]"#,
    );
    assert_eq!(code(&out), 0, "{}", String::from_utf8_lossy(&out.stderr));
    assert_eq!(read(&f), b"1\n2\nthree\n".to_vec());
}

/// Options that were only a second spelling of something else. Each is now a
/// usage error rather than a synonym an agent has to choose between.
#[test]
fn redundant_options_are_gone() {
    let sb = Sandbox::new("removed");
    let f = sb.file("a.txt", b"one\ntwo\n");
    let p = f.to_str().unwrap();

    // `create --overwrite` was exactly `write`.
    assert_eq!(code(&run(&["create", p, "--overwrite", "--text", "x"])), 2);
    // ... and `create` on an existing file points at `write` instead.
    let out = run(&["create", p, "--text", "x"]);
    assert_eq!(code(&out), 7);
    assert!(String::from_utf8_lossy(&out.stderr).contains("intact write"));

    // --text-stdin / --with-stdin are now `--text-file -` / `--with-file -`.
    assert_eq!(code(&run(&["append", p, "--text-stdin"])), 2);
    assert_eq!(
        code(&run(&["replace", p, "--find", "one", "--with-stdin"])),
        2
    );

    // `..` was a second spelling of `:` in a range.
    assert_eq!(code(&run(&["delete", p, "--lines", "1..2"])), 2);

    // `intact encodings` is now the tail of `intact guide encoding`.
    assert_eq!(code(&run(&["encodings"])), 2);
    let guide = stdout(&run(&["guide", "encoding"]));
    assert!(guide.contains("windows-1252"), "no label list");
    assert!(guide.contains("shift_jis"), "no label list");

    // The command aliases still resolve, but are no longer advertised.
    assert_eq!(
        code(&run(&["set-lines", p, "--lines", "1", "--text", "1"])),
        0
    );
    assert!(!stdout(&run(&["--help"])).contains("set-lines"));
    assert!(!stdout(&run(&["--help"])).contains("claude-md"));
}

/// The binary must be self-documenting: everything reachable from --help.
#[test]
fn every_command_has_working_help() {
    let commands = [
        "info",
        "view",
        "search",
        "replace",
        "insert",
        "append",
        "prepend",
        "delete",
        "replace-lines",
        "move-lines",
        "write",
        "create",
        "convert",
        "batch",
        "guide",
        "instructions",
    ];
    let top = run(&["--help"]);
    assert_eq!(code(&top), 0);
    let top_text = stdout(&top);
    for command in commands {
        let out = run(&[command, "--help"]);
        assert_eq!(code(&out), 0, "`{command} --help` failed");
        assert!(
            !stdout(&out).is_empty(),
            "`{command} --help` printed nothing"
        );

        // Every command must be listed in the top-level help, or it is not
        // discoverable from the entry point.
        assert!(
            top_text.contains(command),
            "top-level help does not mention `{command}`"
        );

        // ... and reachable through `intact help CMD` too.
        assert_eq!(code(&run(&["help", command])), 0, "`help {command}` failed");
    }
    // The entry point points at the deeper documentation.
    assert!(top_text.contains("intact guide"));
    assert!(top_text.contains("intact COMMAND --help"));
}

#[test]
fn guide_serves_the_whole_manual() {
    let list = run(&["guide", "--list"]);
    assert_eq!(code(&list), 0);
    let topics = [
        "overview",
        "safety",
        "encoding",
        "ranges",
        "text",
        "exit-codes",
        "json",
        "batch",
        "recipes",
    ];
    for topic in topics {
        assert!(
            stdout(&list).contains(topic),
            "`guide --list` omits `{topic}`"
        );
        let out = run(&["guide", topic]);
        assert_eq!(code(&out), 0, "`guide {topic}` failed");
        assert!(
            stdout(&out).len() > 200,
            "`guide {topic}` is suspiciously short"
        );
    }

    let all = run(&["guide"]);
    assert_eq!(code(&all), 0);
    for topic in topics {
        assert!(stdout(&all).contains(topic), "full manual omits `{topic}`");
    }

    let out = run(&["guide", "nonsense"]);
    assert_eq!(code(&out), 2);
}

/// The manual is one source rendered two ways. This checks the Markdown way
/// carries the same content as the terminal one and is structurally sound —
/// MANUAL.md is this output, and CI only checks that the file matches the
/// binary, not that either is any good.
#[test]
fn guide_renders_the_same_manual_as_markdown() {
    let all = stdout(&run(&["guide", "--markdown"]));
    assert!(all.starts_with("# intact — full manual"));

    // Every topic is present as a real heading rather than fenced text.
    for topic in ["overview", "encoding", "exit-codes", "batch", "recipes"] {
        assert!(
            all.contains(&format!("*`intact guide {topic}`*")),
            "markdown manual omits `{topic}`"
        );
    }

    // Fences have to pair up, or the rest of the file renders as code.
    let fences = all.lines().filter(|l| l.starts_with("```")).count();
    assert_eq!(
        fences % 2,
        0,
        "unbalanced code fences in the markdown manual"
    );

    // Structure that only the block renderer can produce.
    assert!(
        all.contains("| Code | Meaning |"),
        "exit codes are not a table"
    );
    assert!(
        all.contains("### Detection order"),
        "no markdown subheading"
    );
    assert!(
        !all.contains("{#"),
        "raw anchor syntax leaked into the output"
    );

    // Both renderings say the same things, whatever the markup around them.
    for topic in ["overview", "exit-codes"] {
        let one = stdout(&run(&["guide", "--markdown", topic]));
        let plain = stdout(&run(&["guide", topic]));
        for needle in ["intact", "file"] {
            assert!(one.contains(needle) && plain.contains(needle));
        }
        assert!(
            one.len() > 200,
            "`guide --markdown {topic}` is suspiciously short"
        );
    }

    // --markdown is about rendering, and has nothing to say about the index.
    assert_eq!(code(&run(&["guide", "--markdown", "--list"])), 2);
}

#[test]
fn guide_is_available_as_json() {
    let out = run(&["--json", "guide", "ranges"]);
    assert_eq!(code(&out), 0);
    let v: serde_json::Value = serde_json::from_str(&stdout(&out)).unwrap();
    assert_eq!(v["ok"], true);
    assert!(v["content"].as_str().unwrap().contains("1-based"));
    assert_eq!(v["topics"].as_array().unwrap().len(), 9);
}

#[test]
fn instructions_generate_a_pasteable_section() {
    let out = run(&["instructions"]);
    assert_eq!(code(&out), 0);
    let text = stdout(&out);
    assert!(text.starts_with("## "));
    assert!(text.contains("only tool permitted to write to a file"));
    assert!(text.contains("intact replace FILE --find TEXT --with TEXT"));
    assert!(text.contains("intact guide"));

    let brief = stdout(&run(&["instructions", "--brief"]));
    assert!(brief.len() < text.len());
    assert!(brief.contains("only tool permitted to write to a file"));

    // The invocation name is substituted everywhere.
    let renamed = stdout(&run(&["instructions", "--command", "/opt/bin/ie"]));
    assert!(renamed.contains("/opt/bin/ie info FILE"));
    assert!(!renamed.contains("`intact`"));

    // Heading level is respected, for pasting under an existing section.
    assert!(stdout(&run(&["instructions", "--heading-level", "3"])).starts_with("### "));
    assert_eq!(code(&run(&["instructions", "--heading-level", "9"])), 2);

    // The narrower wording still mandates the tool, just for fewer files.
    let legacy = stdout(&run(&["instructions", "--legacy-only"]));
    assert!(legacy.contains("must be made with"));
}

#[test]
fn instructions_can_target_an_agent_calling_into_wsl() {
    let plain = stdout(&run(&["instructions"]));
    assert!(!plain.contains("wsl"));

    let wsl = stdout(&run(&["instructions", "--wsl"]));
    assert!(wsl.contains("### Every command starts with `wsl.exe`"));
    assert!(wsl.contains("wsl.exe intact info FILE"));
    assert!(wsl.contains("wslpath"));
    // The rule is stated once; the command list stays unprefixed and readable.
    assert!(wsl.contains("\nintact info FILE  "));

    // A named distribution is pinned everywhere the launcher appears.
    let distro = stdout(&run(&["instructions", "--wsl", "Ubuntu-24.04"]));
    assert!(distro.contains("wsl.exe -d Ubuntu-24.04 intact info FILE"));
    assert!(!distro.contains("`wsl.exe intact"));

    let brief = stdout(&run(&["instructions", "--brief", "--wsl"]));
    assert!(brief.contains("prefix every command below with `wsl.exe`"));
    assert!(!stdout(&run(&["instructions", "--brief"])).contains("wsl"));
}

#[test]
fn json_error_output_is_parseable() {
    let sb = Sandbox::new("jsonerr");
    let f = sb.file("a.txt", b"x\nx\n");
    let out = run(&[
        "--json",
        "replace",
        f.to_str().unwrap(),
        "--find",
        "x",
        "--with",
        "y",
    ]);
    assert_eq!(code(&out), 4);
    let v: serde_json::Value = serde_json::from_str(&String::from_utf8_lossy(&out.stderr)).unwrap();
    assert_eq!(v["ok"], false);
    assert_eq!(v["kind"], "ambiguous");
}

/// Options that cannot do anything are rejected, rather than accepted and
/// quietly ignored. Each of these used to exit 0 having done nothing, or
/// reported a result that was an artefact of the flag rather than of the file.
#[test]
fn options_that_cannot_apply_are_refused() {
    let sb = Sandbox::new("inapplicable");
    let f = sb.file("a.txt", b"a x b\nc x d\n");
    let p = f.to_str().unwrap();
    let before = read(&f);

    // --no-expand only means something to a regex replacement.
    assert_eq!(
        code(&run(&[
            "replace",
            p,
            "--find",
            "x",
            "--with",
            "y",
            "--no-expand"
        ])),
        2
    );

    // A count of zero: --max 0 reported "no match" on a file full of matches,
    // and --expect 0 could only ever exit 3 (nothing found) or 4 (found).
    assert_eq!(code(&run(&["search", p, "--find", "x", "--max", "0"])), 2);
    for flag in ["--expect", "--occurrence"] {
        assert_eq!(
            code(&run(&[
                "replace", p, "--find", "x", "--with", "y", flag, "0"
            ])),
            2,
            "{flag} 0 was accepted"
        );
    }

    // Where to insert, and what to insert, are both required - and now say so
    // before the file is opened rather than after.
    assert_eq!(code(&run(&["insert", p, "--text", "q"])), 2);
    assert_eq!(code(&run(&["insert", p, "--line", "1"])), 2);
    assert_eq!(code(&run(&["append", p])), 2);
    assert_eq!(code(&run(&["write", p])), 2);

    // Only one argument can read standard input; the second used to get an
    // empty string, turning a replacement into a deletion.
    assert_eq!(
        code(&run(&[
            "replace",
            p,
            "--find-file",
            "-",
            "--with-file",
            "-"
        ])),
        2
    );

    assert_eq!(read(&f), before, "a refused command wrote to the file");
}

/// Only the Unicode encodings have a byte-order mark to add.
#[test]
fn bom_add_needs_an_encoding_that_has_one() {
    let sb = Sandbox::new("bom-add");
    let f = sb.file("a.txt", "café\n".as_bytes());
    let p = f.to_str().unwrap();

    let out = run(&["convert", p, "--to", "windows-1252", "--bom", "add"]);
    assert_eq!(code(&out), 2);
    assert!(String::from_utf8_lossy(&out.stderr).contains("no byte-order mark"));
    assert_eq!(read(&f), "café\n".as_bytes(), "the file was rewritten");

    // ... and still works where there is one.
    assert_eq!(
        code(&run(&["convert", p, "--to", "utf-8", "--bom", "add"])),
        0
    );
    assert!(read(&f).starts_with(&[0xEF, 0xBB, 0xBF]));
}

// UTF-8 that went through a windows-1252 pipe: "café" double-encoded, so the
// text really does read `cafÃ©`, plus a `â€™` from a smart quote.
const DOUBLE_ENCODED: &[u8] = b"caf\xC3\x83\xC2\xA9 l\xC3\xA2\xE2\x82\xAC\xE2\x84\xA2addition\n";

/// Mojibake-shaped text is reported by `info` and warned about before a write,
/// so the damage is visible rather than silently edited around.
#[test]
fn mojibake_is_reported_and_warned_about() {
    let sb = Sandbox::new("mojibake");
    let f = sb.file("a.txt", DOUBLE_ENCODED);
    let p = f.to_str().unwrap();

    let out = run(&["info", p, "--json"]);
    assert_eq!(code(&out), 0);
    let v: serde_json::Value = serde_json::from_str(&stdout(&out)).unwrap();
    assert_eq!(v["mojibake"]["count"], 2);
    assert_eq!(v["mojibake"]["sample"], "Ã©");
    assert_eq!(v["mojibake"]["line"], 1);

    // A write reports it too, and still succeeds: this is an advisory, not a guard.
    let out = run(&["replace", p, "--find", "addition", "--with", "note"]);
    assert_eq!(code(&out), 0);
    let err = String::from_utf8_lossy(&out.stderr);
    assert!(err.contains("mojibake-shaped"), "stderr was: {err}");
    assert!(err.contains("inferred"), "stderr was: {err}");

    // --quiet is about the success line, not about the warning.
    let out = run(&["--quiet", "replace", p, "--find", "note", "--with", "sum"]);
    assert!(stdout(&out).is_empty());
    assert!(String::from_utf8_lossy(&out.stderr).contains("mojibake-shaped"));
}

/// A legacy file read under a correct explicit --encoding whose content was
/// damaged before intact saw it: still flagged, but without the "confirm the
/// encoding" tail, which no longer applies once it has been declared.
#[test]
fn mojibake_in_a_declared_encoding_omits_the_detection_hint() {
    let sb = Sandbox::new("mojibake-declared");
    // windows-1252 bytes that are also valid UTF-8 - the ambiguous case.
    let f = sb.file("a.txt", b"Le caf\xC3\xA9 est pr\xC3\xAAt.\n");
    let p = f.to_str().unwrap();

    let out = run(&["info", p, "--encoding", "windows-1252", "--json"]);
    let v: serde_json::Value = serde_json::from_str(&stdout(&out)).unwrap();
    assert_eq!(v["mojibake"]["count"], 2);

    let out = run(&[
        "--encoding",
        "windows-1252",
        "replace",
        p,
        "--find",
        "est",
        "--with",
        "reste",
    ]);
    let err = String::from_utf8_lossy(&out.stderr);
    assert!(err.contains("mojibake-shaped"), "stderr was: {err}");
    assert!(!err.contains("inferred"), "stderr was: {err}");

    // Read as UTF-8 the same bytes are clean text, so nothing is reported.
    // Nothing in the bytes distinguishes the two readings; only --encoding does.
    let out = run(&["info", p, "--json"]);
    let v: serde_json::Value = serde_json::from_str(&stdout(&out)).unwrap();
    assert_eq!(v["detected_by"], "utf-8-valid");
    assert!(v.get("mojibake").is_none());
}

/// The check must stay quiet on genuine text that merely contains the lead
/// characters, or the warning becomes noise people learn to ignore.
#[test]
fn ordinary_accented_text_is_not_flagged_as_mojibake() {
    let sb = Sandbox::new("mojibake-clean");
    for (name, bytes) in [
        ("pt.txt", "SÃO PAULO, região\n".as_bytes()),
        ("fr.txt", "Un château, une âme, Âgé\n".as_bytes()),
        ("mix.txt", "Ãtta ÂB Ãx â‚ é ü ñ\n".as_bytes()),
    ] {
        let f = sb.file(name, bytes);
        let out = run(&["info", f.to_str().unwrap(), "--json"]);
        let v: serde_json::Value = serde_json::from_str(&stdout(&out)).unwrap();
        assert!(v.get("mojibake").is_none(), "{name} was flagged: {v}");
    }
}
