//! The manual text itself: one section per `intact guide TOPIC`.
//!
//! Content only — everything that renders it lives in `block.rs`. A body is a
//! list of blocks so that structure is declared rather than inferred from
//! indentation, which is what lets the same source print as terminal text and
//! as Markdown.

use super::{Block, Section};

pub const SECTIONS: &[Section] = &[
    Section {
        key: "overview",
        title: "OVERVIEW",
        summary: "what intact is for and how a session typically goes",
        body: &[
            Block::Prose(
                "intact edits text files driven entirely by command-line arguments, without ever\n\
                 changing a file's encoding.",
            ),
            Block::Prose(
                "It exists for AI coding agents. An agent works in UTF-8 internally, so writing a\n\
                 file back naively re-encodes everything as UTF-8 and turns a Latin-1 `café` into\n\
                 `cafÃ©`. intact takes UTF-8 text on the command line, transcodes it into\n\
                 whatever encoding the target file already uses, and splices it in without\n\
                 touching any other byte.",
            ),
            Block::Prose("A typical session:"),
            Block::Code {
                lang: "bash",
                text: r#"intact info src/legacy.c              # what encoding is this, is it safe?
intact view src/legacy.c --lines 40:80 --number
intact search src/legacy.c --find 'malloc('
intact replace src/legacy.c --find 'malloc(n)' --with 'calloc(n, 1)'"#,
            },
            Block::Prose(
                "Every command accepts `--help`. `intact guide` prints this whole manual;\n\
                 `intact guide TOPIC` prints one section; `intact guide --list` names them.",
            ),
            Block::Heading("COMMANDS"),
            // Three captioned tables rather than one with a group column: the
            // grouping is the point, and a reader scanning for "can this write
            // to my file?" gets the answer from the caption alone.
            Block::Table {
                caption: "Inspect (never write)",
                head: &[],
                rows: &[
                    &[
                        "info",
                        "encoding, BOM, line endings, line count, edit safety",
                    ],
                    &["view", "print a file or a line range as UTF-8"],
                    &[
                        "search",
                        "find a string or regex, with line and column numbers",
                    ],
                ],
            },
            Block::Table {
                caption: "Edit",
                head: &[],
                rows: &[
                    &["replace", "replace occurrences of a string or regex"],
                    &["insert", "insert text before or after a line"],
                    &["append", "add text at the end of the file"],
                    &["prepend", "add text at the start of the file"],
                    &["delete", "delete a line range"],
                    &["replace-lines", "replace a line range with new text"],
                    &["move-lines", "move a line range elsewhere in the same file"],
                    &["write", "replace the entire contents, keeping the encoding"],
                    &["create", "create a new file, failing if it exists"],
                    &["convert", "re-encode the file into a different encoding"],
                    &[
                        "batch",
                        "several operations, one write per file — and the only command that can edit more than one file",
                    ],
                ],
            },
            Block::Table {
                caption: "Documentation",
                head: &[],
                rows: &[
                    &["guide", "this manual"],
                    &[
                        "instructions",
                        "a section to paste into a project's CLAUDE.md / AGENTS.md",
                    ],
                    &["help", "per-command help (also `intact CMD --help`)"],
                ],
            },
            Block::Heading("REACH FOR BATCH WHEN THERE IS MORE THAN ONE EDIT"),
            Block::Prose(
                "A file that needs several changes wants one `intact batch` rather than a run of\n\
                 separate commands: the operations are described in a JSON script, applied in\n\
                 order, and written once. Nothing is written unless every one of them succeeds,\n\
                 so a script cannot leave a file half-edited. Operations may name a \"file\" of\n\
                 their own, which is how one command edits several files. See `intact guide\n\
                 batch`.",
            ),
            Block::Heading("SCOPE"),
            Block::Prose(
                "intact writes file contents: it creates, edits and truncates files, and\n\
                 `--parents` will create a missing directory for a new file. It deliberately\n\
                 does not delete, rename, move or copy files, and does not change permissions —\n\
                 use the normal tools for those.",
            ),
            Block::Prose(
                "Every command but `batch` takes exactly one file, so that each file's encoding\n\
                 is decided and reported separately. There is no glob: `batch` names its files\n\
                 explicitly, and a shell loop covers the rest.",
            ),
            Block::Prose(
                "Everything is configured by command-line flags. intact reads no environment\n\
                 variables and no configuration file, because a tool that is usually driven one\n\
                 command per shell cannot rely on state surviving between them.",
            ),
        ],
    },
    Section {
        key: "safety",
        title: "SAFETY MODEL",
        summary: "the guarantees, and what makes a command refuse to write",
        body: &[
            Block::Heading("GUARANTEES"),
            Block::Numbered(&[
                "Encoding is preserved. The file's encoding is detected (BOM, then valid\n\
                 UTF-8, then statistical detection) or forced with `--encoding`. Output is\n\
                 written in that same encoding.",
                "Untouched bytes stay identical. When the file round-trips through its\n\
                 encoding, edits are applied by splicing encoded bytes into the original\n\
                 buffer. Everything outside the edited region is the byte that was already\n\
                 there, so repeated edits cannot accumulate mojibake.",
                "No silent corruption. If the file does not round-trip, or if your text\n\
                 contains a character the file's encoding cannot represent, the command fails\n\
                 with a distinct exit code and writes nothing. A file that does not read as\n\
                 text is refused outright, by the reading commands as much as the writing\n\
                 ones: `view` on a binary would otherwise pour NULs and escape sequences into\n\
                 your terminal and report nothing wrong.",
                "Line endings and BOMs are preserved. Inserted text is rewritten to the\n\
                 file's dominant line ending; a BOM stays exactly as it was; a missing final\n\
                 newline stays missing.",
                "Writes are atomic. Content goes to a temporary file in the same directory,\n\
                 then is renamed over the original, carrying its permissions. Editing a\n\
                 symlink writes the file it points at and leaves the link intact; the result\n\
                 line names both (`link.txt -> real.txt: updated ...`), so an edit landing\n\
                 somewhere other than the path you typed is never silent.",
            ]),
            Block::Heading("WHEN A COMMAND REFUSES"),
            Block::Code {
                lang: "text",
                text: r#"refusing to edit: FILE does not round-trip through ENCODING"#,
            },
            Block::Prose(
                "Detection picked an encoding that cannot reproduce the original bytes, or the\n\
                 file is damaged. Run `intact info FILE` and pass the right `--encoding`.\n\
                 `--lossy` rewrites the whole file anyway, accepting that untouched parts may\n\
                 change. Exit code 5.",
            ),
            Block::Code {
                lang: "text",
                text: r#"character 'X' (U+NNNN) cannot be represented in ENCODING"#,
            },
            Block::Prose(
                "Your replacement text needs a character the file's encoding lacks. Either\n\
                 `intact convert FILE --to utf-8` first, or pick a substitution policy with\n\
                 `--unmappable replace|xml|skip`. Exit code 5.",
            ),
            Block::Code {
                lang: "text",
                text: r#"N occurrences of text "..."; refusing to guess"#,
            },
            Block::Prose(
                "replace requires a unique match by default. Pass `--all`, `--occurrence N`,\n\
                 `--lines RANGE`, or extend `--find` until it is unique. Exit code 4.",
            ),
            Block::Code {
                lang: "text",
                text: r#"FILE does not look like a text file: REASON"#,
            },
            Block::Prose(
                "The first 8 KiB of the file do not read as text: a NUL byte, a high proportion\n\
                 of control characters, or - in UTF-16 - an unpaired surrogate. This guard\n\
                 covers reading as well as writing, so `view` and `search` refuse too; `info` is\n\
                 the exception and always reports. Pass `--force` if you really mean it. Exit\n\
                 code 5.",
            ),
            Block::Prose(
                "`info` reports such a file as its bytes and a verdict — path, size, and a `not\n\
                 text:` line naming the reason — and withholds the rest. Everything below\n\
                 `bytes` describes the file decoded as text: an encoding detection run over it,\n\
                 the line endings of the result, how many characters it came to. None of that is\n\
                 a fact about a file that is not text. The encoding is a guess about a blob, and\n\
                 the line endings are however many 0x0A bytes happened to fall in it. `info\n\
                 --force` prints the full report anyway, `--force` meaning here what it means\n\
                 everywhere else.",
            ),
            Block::Prose(
                "One REASON has a better answer than `--force`: \"a NUL every other byte, which\n\
                 is how UTF-16LE with no BOM reads as single bytes\" means the file is text that\n\
                 nothing declared the encoding of. Detection cannot find it unaided. Pass\n\
                 `--encoding utf-16le` (or utf-16be) and it reads normally.",
            ),
            Block::Heading("SHOWING THE CHANGE"),
            Block::Prose(
                "Two flags print a unified diff of an edit. `--dry-run` prints it and writes\n\
                 nothing; `--show-diff` prints it and applies the edit anyway.",
            ),
            Block::Code {
                lang: "bash",
                text: r#"intact --dry-run replace app.py --find x --with y --all   # preview only
intact --show-diff replace app.py --find x --with y --all # apply, and show"#,
            },
            Block::Prose(
                "Prefer `--show-diff` when the point is for someone to see what happened. A\n\
                 preview followed by the real command is two invocations of two different\n\
                 commands, and the file can differ between them; one invocation that reports\n\
                 exactly what it changed cannot drift. `--dry-run` is for deciding whether to\n\
                 make the edit at all.",
            ),
            Block::Prose(
                "`--diff-context N` (default 3) sets how many unchanged lines are shown either\n\
                 side of a change.",
            ),
            Block::Prose(
                "Global flags are accepted before or after the subcommand, but write them\n\
                 before it:",
            ),
            Block::Code {
                lang: "bash",
                text: r#"intact --dry-run replace app.py --find x --with y      # do this
intact replace app.py --find x --with y --dry-run      # not this"#,
            },
            Block::Prose(
                "Only the first form can be matched by a tool that allows commands by prefix.\n\
                 Under an agent harness that asks permission per command, a rule matching\n\
                 `intact --dry-run ` pre-approves every preview while leaving real writes to\n\
                 prompt — which only works if the flag is where a prefix can see it.",
            ),
            Block::Prose(
                "The output is a real unified diff, so it can be piped anywhere that reads\n\
                 one. With `--quiet`, which suppresses only the summary line, stdout is the\n\
                 patch and nothing else:",
            ),
            Block::Code {
                lang: "bash",
                text: r#"intact --dry-run --quiet replace app.py --find x --with y > change.patch
git apply -p0 --check change.patch"#,
            },
            Block::Prose(
                "Line terminators are deliberately not compared line by line — converting a\n\
                 file from LF to CRLF would otherwise report every line as changed. They are\n\
                 reported above the diff instead, whenever the styles in use change:",
            ),
            Block::Code {
                lang: "text",
                text: r#"# line endings: lf=3 crlf=0 cr=0 -> lf=3 crlf=1 cr=0"#,
            },
            Block::Prose(
                "That line is how appending CRLF text to an LF file becomes visible. A change\n\
                 with no textual difference at all — re-encoding a file, adding a BOM — says\n\
                 so rather than printing nothing.",
            ),
        ],
    },
    Section {
        key: "encoding",
        title: "ENCODINGS",
        summary: "detection, labels, and forcing an encoding",
        body: &[
            Block::Heading("DETECTION ORDER"),
            Block::Numbered(&[
                "`--encoding LABEL`, if given, wins outright.",
                "A byte-order mark (UTF-8, UTF-16LE, UTF-16BE).",
                "Bytes that are valid UTF-8 are treated as UTF-8.",
                "Otherwise chardetng guesses a legacy encoding.",
            ]),
            Block::Prose(
                "`intact info FILE` reports which of these applied, as `detected_by`:\n\
                 explicit, bom, utf-8-valid, guessed, or default (empty/new file).",
            ),
            Block::Prose(
                "Step 3 is a validity check, not a verification. Every single-byte encoding\n\
                 decodes every byte sequence, so bytes that are valid UTF-8 may equally be a\n\
                 windows-1252 file whose own content is already mojibake: `Ã©` in windows-1252 is\n\
                 the bytes C3 A9, which are also a perfectly good UTF-8 `é`. Such a file is\n\
                 detected as UTF-8 and edited as UTF-8, and the text you insert is then wrong for\n\
                 every reader that opens it as windows-1252.",
            ),
            Block::Prose(
                "Nothing detects this, and no future version will. A UTF-8 file containing `café`\n\
                 and a windows-1252 file containing `cafÃ©` are the same 5 bytes; the difference\n\
                 is intent, which is not in the file. Only `--encoding` settles it. This is the\n\
                 one case where the guess is silently wrong rather than loudly wrong, and it is\n\
                 why a project with a known encoding should declare it on every command.",
            ),
            Block::Prose(
                "Note the limit of that risk: a windows-1252 file with ordinary text is not\n\
                 affected. Real `café` is 63 61 66 E9, which is not valid UTF-8, so it reaches\n\
                 chardetng at step 4 as intended. Only a file whose windows-1252 content is\n\
                 *itself* mojibake reaches step 3 by accident — a file that was already damaged\n\
                 before `intact` saw it.",
            ),
            Block::Heading("MOJIBAKE ALREADY IN THE FILE"),
            Block::Prose(
                "Text that has been through the wrong encoding leaves a recognisable shape: `Ã©`\n\
                 where `é` was meant, `â€™` where a right single quote was. `intact info` reports\n\
                 it as a warning line, and every command that writes prints the same warning on\n\
                 stderr before its result:",
            ),
            Block::Code {
                lang: "bash",
                text: r#"intact: warning: app.py: 2 mojibake-shaped sequence(s), first "Ã©" at
line 1: text that was written through the wrong encoding at some point."#,
            },
            Block::Prose(
                "This is an advisory, not a guard: the edit goes through, and `--quiet` does not\n\
                 silence it. Under `--json` it is a `mojibake` object on an `info` result and a\n\
                 `warnings` array on an edit result — see `intact guide json`.",
            ),
            Block::Prose(
                "It reports damage, not misdetection, and cannot report the detection case above:\n\
                 a windows-1252 file whose bytes are valid UTF-8 decodes to clean text, so there\n\
                 is no shape to see. When the encoding was inferred rather than declared the\n\
                 warning says so and asks for `--encoding LABEL`, because a wrong reading and\n\
                 real damage look alike from here.",
            ),
            Block::Prose(
                "Do not hand-fix the characters. Rewriting one `Ã©` as `é` repairs a single\n\
                 occurrence and leaves the rest of the file as it was; the repair is to re-encode\n\
                 the whole file from the encoding it was mangled through, which is a decision for\n\
                 whoever owns the file.",
            ),
            Block::Heading("PROJECTS THAT MANDATE ONE ENCODING"),
            Block::Prose(
                "If every file in a project must be, say, Latin-1, do not rely on detection at\n\
                 all. Pass both flags on every command that writes:",
            ),
            Block::Code {
                lang: "bash",
                text: r#"intact --encoding latin1 --no-guess replace FILE --find X --with Y"#,
            },
            Block::Prose(
                "`--encoding` applies to `create` too, which otherwise makes UTF-8 files.",
            ),
            Block::Prose(
                "`--no-guess` makes any *write* to a file whose encoding was merely guessed fail\n\
                 with exit 5 instead of proceeding. Read-only commands (info, view, search)\n\
                 still work, so a file that trips the guard can still be diagnosed.",
            ),
            Block::Prose(
                "Note its scope: `--no-guess` covers `detected_by: guessed`, the chardetng path.\n\
                 It does not fire on `utf-8-valid`, which is an inference too but not a\n\
                 statistical one. Only `--encoding` covers that case, which is why the two flags\n\
                 go together rather than either standing in for the other.",
            ),
            Block::Prose(
                "That pairing matters more than it looks. A wrong single-byte guess does not\n\
                 merely display the file oddly: existing bytes survive, but text you insert is\n\
                 encoded in the wrong repertoire. Inserting 'ć' into a file guessed as\n\
                 windows-1250 writes byte 0xE6, which a Latin-1 reader shows as 'æ'. Declaring\n\
                 the encoding removes that whole class of failure.",
            ),
            Block::Prose(
                "There is no environment variable or config file for this, deliberately. A tool\n\
                 driven one command per shell — which is how an agent runs it — cannot rely on an\n\
                 `export` from a previous command still being set, and a mandate that applies\n\
                 only sometimes is worse than none. Put the flags in the command.",
            ),
            Block::Prose(
                "`intact instructions --encoding latin1` generates a CLAUDE.md section stating\n\
                 the policy, for agents working in such a project.",
            ),
            Block::Heading("GUESSES ARE GUESSES"),
            Block::Prose(
                "Statistical detection needs a reasonable amount of text. On a short file with\n\
                 one or two non-ASCII bytes it may land on the wrong single-byte encoding —\n\
                 windows-1250 instead of windows-1252, say. Bytes already in the file are still\n\
                 safe, because every single-byte encoding round-trips and nothing existing gets\n\
                 rewritten. But the character repertoire differs, so inserting 'ã' into a file\n\
                 believed to be windows-1250 fails with exit 5 and a message saying the encoding\n\
                 was guessed rather than declared.",
            ),
            Block::Prose(
                "When a project's encoding is known, pass `--encoding`, and add `--no-guess` so\n\
                 a missed flag fails loudly instead of falling back to a guess.",
            ),
            Block::Heading("LABELS"),
            Block::Prose(
                "Labels follow the WHATWG Encoding Standard: utf-8, utf-16le, utf-16be,\n\
                 windows-1250 through windows-1258, windows-874, iso-8859-2 through iso-8859-16,\n\
                 koi8-r, koi8-u, macintosh, x-mac-cyrillic, ibm866, gbk, gb18030, big5, euc-jp,\n\
                 shift_jis, iso-2022-jp, euc-kr, and their usual aliases (latin1, cp1252, ...).\n\
                 The complete list this build accepts is at the end of this topic.",
            ),
            Block::Prose("Two things worth knowing:"),
            Block::Bullets(&[
                "Per the standard, latin1 / iso-8859-1 resolve to windows-1252, which differs\n\
                 from strict ISO 8859-1 only in how bytes 0x80-0x9F are named. Both\n\
                 round-trip, so the bytes on disk are unaffected either way.",
                "iso-2022-jp is stateful, so byte-exact splicing is unavailable for it. Such\n\
                 files need `--lossy`, which re-encodes the whole file.",
            ]),
            Block::Heading("CONVERTING"),
            Block::Code {
                lang: "bash",
                text: r#"intact convert FILE --to utf-8
intact convert FILE --to utf-8 --bom remove
intact convert FILE --to utf-16le            # a BOM is added automatically
intact convert FILE --to utf-8 --newlines lf # also normalise line endings"#,
            },
            Block::Prose(
                "`--bom keep` (default) preserves whether the file had one; add and remove force\n\
                 it. UTF-16 output always gets a BOM, since UTF-16 without one is undetectable.",
            ),
            // The label list used to be a command of its own (`intact
            // encodings`) and then a special case in the renderer. It is
            // neither now: the build's own table is simply a block of this
            // section, which is where a reader looks for it anyway.
            Block::Heading("EVERY LABEL THIS BUILD ACCEPTS"),
            Block::Labels(crate::encoding_util::KNOWN_LABELS),
            Block::Prose(
                "Aliases such as latin1, latin-1, iso-8859-1, cp1252 and ansi_x3.4-1968 are\n\
                 accepted as well.",
            ),
        ],
    },
    Section {
        key: "ranges",
        title: "LINE NUMBERS AND RANGES",
        summary: "the syntax accepted by --lines, --line, --after and --before",
        body: &[
            Block::Prose("Line numbers are 1-based. Ranges are inclusive at both ends."),
            Block::Table {
                caption: "",
                head: &["Range", "Means"],
                rows: &[
                    &["7", "line 7"],
                    &["5:9", "lines 5 through 9"],
                    &["5:", "line 5 to the end of the file"],
                    &[":9", "the start of the file through line 9"],
                    &["$", "the last line (`end` and `last` also work)"],
                    &["3:$", "line 3 to the last line"],
                    &["-1", "the last line"],
                    &["-3:-1", "the last three lines"],
                ],
            },
            Block::Prose("`:` is the only range separator."),
            Block::Prose(
                "`--line`, `--after` and `--before` take a single position (7, $, -2); `--lines`\n\
                 takes a range.",
            ),
            Block::Prose(
                "`insert --line N` and `move-lines --before N` accept one past the last line,\n\
                 meaning \"start a new line at the end of the file\". `insert --after N` and\n\
                 `move-lines --after N` require an existing line.",
            ),
            Block::Prose(
                "A range that runs backwards, or names a line beyond the end of the file, exits 6.",
            ),
            Block::Prose(
                "Commands that accept `--lines` as a filter rather than a target — replace and\n\
                 search — restrict the operation to that region and leave the rest alone.",
            ),
            Block::Prose(
                "A destination named for `move-lines` is a line of the file as it is numbered\n\
                 now, not as it will be numbered once the block has been lifted out of it.",
            ),
        ],
    },
    Section {
        key: "text",
        title: "SUPPLYING TEXT",
        summary: "--text, --text-file, and backslash escapes",
        body: &[
            Block::Prose(
                "Input text is always UTF-8. It is transcoded into the file's own encoding on\n\
                 write, which is the entire point of this tool.",
            ),
            Block::Prose("Every command that takes text accepts one of:"),
            Block::Table {
                caption: "",
                head: &["Flag", "Takes"],
                rows: &[
                    &["--text TEXT, -t", "the argument itself"],
                    &["--text-file PATH", "a UTF-8 file; `-` means standard input"],
                ],
            },
            Block::Prose("replace uses a matching pair for each half:"),
            Block::Table {
                caption: "",
                head: &["Find", "With"],
                rows: &[
                    &["--find TEXT, -f", "--with TEXT, -w"],
                    &["--find-file PATH", "--with-file PATH"],
                    &["", "--delete — remove the match instead of replacing it"],
                ],
            },
            Block::Prose(
                "Any path argument that reads text accepts `-` for standard input, including\n\
                 `batch --script -`. There is no separate `--text-stdin` flag.",
            ),
            Block::Heading("ESCAPES"),
            Block::Prose(
                "`--escapes` interprets backslash sequences in `--text`, `--find` and `--with`.\n\
                 This is the easiest way to pass multi-line content in a single argument:",
            ),
            Block::Code {
                lang: "bash",
                text: r#"intact insert main.rs --line 1 --escapes --text 'use std::fmt;\nuse std::io;'"#,
            },
            Block::Prose("Supported: \\n \\r \\t \\0 \\\\ \\' \\\" \\xNN \\uXXXX \\u{XXXXX}"),
            Block::Prose(
                "It applies to the text whatever it came from, so `--text-file`, `--find-file`\n\
                 and `--with-file` content is unescaped too. That is worth knowing before\n\
                 combining `--escapes` with a file: a block holding a Windows path or a regex\n\
                 has its backslashes interpreted like any other. Text in a `batch` script is the\n\
                 exception - JSON has escapes of its own and `--escapes` never touches it.",
            ),
            Block::Prose(
                "Without `--escapes`, a literal backslash in your text is just a backslash,\n\
                 which is what you want when editing code containing regex or Windows paths.",
            ),
            Block::Heading("LINE ENDINGS IN INSERTED TEXT"),
            Block::Prose("`--eol` controls the terminators of text you supply:"),
            Block::Table {
                caption: "",
                head: &["--eol", "Effect"],
                rows: &[
                    &[
                        "auto",
                        "(default) rewrite them to match the file's dominant line ending",
                    ],
                    &["lf, crlf, cr", "force one"],
                    &["keep", "insert the text exactly as given"],
                ],
            },
            Block::Prose(
                "Under auto, a literal (non-regex) `--find` is also rewritten, so searching for\n\
                 'a\\nb' works on a CRLF file.",
            ),
            Block::Heading("PROJECTS THAT MANDATE ONE LINE-ENDING STYLE"),
            Block::Prose(
                "auto is right for a repository of mixed files and wrong for one with a policy:\n\
                 it follows each file rather than the policy, and a brand-new file created with\n\
                 auto always gets LF. As with encodings, say so on the command:",
            ),
            Block::Code {
                lang: "bash",
                text: r#"intact --eol crlf --strict-eol append FILE --text TEXT"#,
            },
            Block::Prose(
                "`--eol` applies to `create` too, so a new file gets the mandated style rather\n\
                 than LF.",
            ),
            Block::Prose(
                "`--strict-eol` makes any write to a file whose existing terminators are not\n\
                 the mandated ones fail with exit 5, instead of appending CRLF text to an LF\n\
                 file and leaving it mixed. It reports the counts it found:",
            ),
            Block::Code {
                lang: "text",
                text: r#"refusing to write: app.c has 2 line ending(s) that are not CRLF
(lf=2, crlf=0, cr=0)
hint: normalise it first: `intact convert app.c --newlines crlf`"#,
            },
            Block::Prose(
                "The guard needs something to enforce, so `--eol` must be lf, crlf or cr; with\n\
                 auto or keep it is a usage error rather than a silent no-op. write, create and\n\
                 convert are exempt, because they produce compliant output whatever the file\n\
                 held before.",
            ),
            Block::Heading("NORMALISING AN EXISTING FILE"),
            Block::Code {
                lang: "bash",
                text: r#"intact convert FILE --newlines crlf   # --to is optional here
intact convert FILE --newlines auto   # collapse mixed endings to the
                                        # file's own dominant style"#,
            },
            Block::Prose(
                "`--to` may be omitted when `--newlines` is given, so line endings can be fixed\n\
                 without restating (or knowing) the file's encoding.",
            ),
            Block::Heading("TRAILING NEWLINES"),
            Block::Prose(
                "append, prepend, write and create ensure the result ends with a line terminator;\n\
                 `--no-trailing-newline` turns that off. replace-lines keeps whatever the\n\
                 replaced region had, so replacing the last line of a file that lacks a final\n\
                 newline does not add one. move-lines keeps it too, from either end: a block\n\
                 lifted from an unterminated last line takes the terminator above it along and\n\
                 gains one of its own, and a block landing after an unterminated last line gives\n\
                 its own up.",
            ),
        ],
    },
    Section {
        key: "exit-codes",
        title: "EXIT CODES",
        summary: "one code per failure class, for branching without parsing prose",
        body: &[
            // Headed, unlike the command tables: a bare number means nothing
            // without the word "code" next to it.
            Block::Table {
                caption: "",
                head: &["Code", "Meaning"],
                rows: &[
                    &["0", "success"],
                    &["1", "generic failure"],
                    &["2", "bad arguments"],
                    &["3", "no match / target not found"],
                    &["4", "ambiguous match (more occurrences than allowed)"],
                    &[
                        "5",
                        "encoding problem (undecodable file, or text unrepresentable in it)",
                    ],
                    &["6", "line number or range out of bounds"],
                    &["7", "file already exists (create)"],
                    &["8", "file not found"],
                    &["9", "I/O error"],
                ],
            },
            Block::Prose(
                "`search` exits 3 when nothing matched; pass `--allow-empty` for exit 0 instead.",
            ),
            Block::Prose(
                "Every failing command writes nothing. A refusal never leaves the file in a\n\
                 half-edited state, and batch is all-or-nothing across all of its operations.",
            ),
        ],
    },
    Section {
        key: "json",
        title: "JSON OUTPUT",
        summary: "machine-readable results and errors",
        body: &[
            Block::Prose(
                "`--json` makes every command that reports a result emit one JSON object.\n\
                 Successful results go to stdout, failures to stderr, and the process exit code\n\
                 is unchanged. (`instructions` is the exception: it prints Markdown for a\n\
                 CLAUDE.md whatever else is passed.)",
            ),
            Block::Heading("SUCCESS (edits)"),
            Block::Code {
                lang: "json",
                text: r#"{"ok":true,"command":"replace","path":"a.txt","encoding":"windows-1252",
 "detected_by":"guessed","bom":false,"eol":"lf","changed":true,
 "dry_run":false,"bytes_before":30,"bytes_after":31,"lines_before":3,
 "lines_after":3,"summary":"replaced 1 of 1 occurrence(s)",
 "occurrences_found":1,"occurrences_replaced":1,"first_line":1,
 "first_column":4}"#,
            },
            Block::Prose("Command-specific fields are merged into the same object."),
            Block::Prose(
                "Every edit result carries \"edits\": the spans that were replaced, each with\n\
                 \"line\", \"column\", \"end_line\", \"end_column\", \"offset\", \"end_offset\",\n\
                 \"before\" and \"after\". That is what intact actually did, rather than what\n\
                 comparing two versions of the file suggests it did, and it is the field to\n\
                 read when a caller needs to know where an edit landed. Long text is cut to\n\
                 400 characters with \"truncated\":true; at most 200 edits are listed, and\n\
                 \"edit_count\" is always the real total.",
            ),
            Block::Code {
                lang: "json",
                text: r#""edit_count":1,"edits":[{"line":2,"column":1,"end_line":2,
 "end_column":5,"offset":6,"end_offset":10,"before":"beta",
 "after":"BETA","truncated":false}]"#,
            },
            Block::Prose(
                "`--dry-run` and `--show-diff` add a \"diff\" field holding the complete unified\n\
                 diff — never truncated, unlike the human output — plus \"eol_before\" and\n\
                 \"eol_after\" when the line-ending styles change.",
            ),
            Block::Prose(
                "A \"resolved_path\" field appears (in edit and info results alike) only when\n\
                 the path given is a symlink, and holds the file actually read and written.\n\
                 Its absence means the path is the file.",
            ),
            Block::Prose(
                "A \"warnings\" array appears, on the same terms, when something about the file\n\
                 deserves saying without blocking the write — today, mojibake-shaped text in it.\n\
                 Its absence means there was nothing to say. Outside `--json` the same warnings\n\
                 go to stderr, and `--quiet` does not suppress them.",
            ),
            Block::Code {
                lang: "json",
                text: r#""warnings":["2 mojibake-shaped sequence(s), first \"Ã©\" at line 1: ..."]"#,
            },
            Block::Heading("SUCCESS (batch)"),
            Block::Prose(
                "batch reports per file, so its result carries a \"files\" array instead of the\n\
                 single-file fields above. The array is always present, whatever the count.",
            ),
            Block::Code {
                lang: "json",
                text: r#"{"ok":true,"command":"batch","operations":3,"changed":true,
 "dry_run":false,"files":[
   {"path":"a.py","encoding":"UTF-8","detected_by":"utf-8-valid",
    "bom":false,"eol":"lf","changed":true,"dry_run":false,
    "bytes_before":120,"bytes_after":118,"lines_before":9,
    "lines_after":9,"summary":"applied 2 operation(s)","operations":2}]}"#,
            },
            Block::Heading("SUCCESS (search)"),
            Block::Code {
                lang: "json",
                text: r#"{"ok":true,"command":"search","path":"a.txt",
 "encoding":"windows-1252","count":2,"matches":[
   {"line":1,"column":4,"offset":3,"match":"é","text":"café"}]}"#,
            },
            Block::Heading("SUCCESS (info)"),
            Block::Code {
                lang: "json",
                text: r#"{"ok":true,"command":"info","path":"a.txt","bytes":30,
 "encoding":"windows-1252","detected_by":"guessed","bom":false,
 "eol":"lf","eol_counts":{"lf":3,"crlf":0,"cr":0},"lines":3,
 "characters":30,"ends_with_newline":true,"decode_errors":false,
 "roundtrip_safe":true,"looks_binary":false}"#,
            },
            Block::Prose(
                "`info` carries no \"warnings\": its equivalent is two optional objects, each\n\
                 present only when it applies, and never both — mojibake is not reported for a\n\
                 file that is not text. \"mojibake\" reports mojibake-shaped sequences:",
            ),
            Block::Code {
                lang: "json",
                text: r#""mojibake":{"count":2,"line":1,"sample":"Ã©"}"#,
            },
            Block::Prose(
                "and \"binary\" says why the file was judged not to be text, which is why every\n\
                 other command refuses it. \"reason\" is one of nul, utf-16-no-bom,\n\
                 unpaired-surrogate or controls; \"offset\" is null where the judgement rests on\n\
                 a proportion rather than one position.",
            ),
            Block::Code {
                lang: "json",
                text: r#""binary":{"reason":"nul","offset":7,"detail":"NUL byte at offset 7 (0x7)"}"#,
            },
            Block::Prose(
                "When \"binary\" is present, every field derived from decoding the file is\n\
                 absent, because none of them describes the file. Test for \"binary\" (or read\n\
                 \"looks_binary\", which is always present) before reading \"encoding\",\n\
                 \"eol\", \"lines\" or the rest, and expect this shape:",
            ),
            Block::Code {
                lang: "json",
                text: r#"{"ok":true,"command":"info","path":"a.bin","bytes":142312,
 "looks_binary":true,"binary":{...}}"#,
            },
            Block::Prose(
                "Adding `--force` decodes it as text regardless and returns the full object,\n\
                 the \"binary\" object included.",
            ),
            Block::Heading("FAILURE (on stderr)"),
            Block::Code {
                lang: "json",
                text: r#"{"ok":false,"error":"...","kind":"ambiguous","hint":"...","exit_code":4}"#,
            },
            Block::Prose(
                "\"kind\" is one of: other, usage, no_match, ambiguous, encoding, range, exists,\n\
                 not_found, io — the same taxonomy as the exit codes.",
            ),
        ],
    },
    Section {
        key: "batch",
        title: "BATCH SCRIPTS",
        summary: "several edits and several files, all-or-nothing",
        body: &[
            Block::Code {
                lang: "bash",
                text: r#"intact batch FILE --script ops.json
intact batch FILE --script -        # read the script from stdin
intact batch --script ops.json      # every op names its own file"#,
            },
            Block::Prose(
                "Use this whenever a file needs more than one change. Operations are applied in\n\
                 order and later ones see the results of earlier ones, so line numbers refer to\n\
                 the state at that step. If any operation fails, nothing is written at all — not\n\
                 for that file and not for any other.",
            ),
            Block::Code {
                lang: "json",
                text: r##"{
  "ops": [
    { "op": "replace", "find": "DEBUG = True", "with": "DEBUG = False" },
    { "op": "replace", "find": "log(", "with": "logger.info(", "all": true },
    { "op": "delete", "lines": "40:42" },
    { "op": "insert", "line": 1, "text": "# generated" },
    { "op": "append", "text": "# end" }
  ]
}"##,
            },
            Block::Prose("A bare JSON array works too. Fields per op:"),
            Block::Table {
                caption: "",
                head: &["Op", "Fields"],
                rows: &[
                    &[
                        "replace",
                        "find, with, regex, ignore_case, all, occurrence, expect, lines, no_expand",
                    ],
                    &["insert", "line or after, text"],
                    &["append", "text"],
                    &["prepend", "text"],
                    &["delete", "lines"],
                    &["replace-lines", "lines, text"],
                    &["move-lines", "lines, and one of after, before or by"],
                    &["write", "text"],
                ],
            },
            Block::Prose(
                "Every op also accepts \"file\". \"lines\", \"line\", \"after\" and \"before\" accept a\n\
                 number or any range string from the ranges topic. Unknown fields are rejected, so a typo fails\n\
                 loudly instead of being ignored.",
            ),
            Block::Prose(
                "Text in a script is UTF-8 JSON, with normal JSON escapes — use \\n for newlines\n\
                 rather than the `--escapes` flag.",
            ),
            Block::Heading("SEVERAL FILES IN ONE COMMAND"),
            Block::Prose(
                "An operation's \"file\" says which file it edits. FILE on the command line is the\n\
                 default for operations that do not name one, and may be omitted entirely when\n\
                 they all do.",
            ),
            Block::Code {
                lang: "json",
                text: r#"{
  "ops": [
    { "op": "replace", "file": "src/a.py", "find": "old", "with": "new", "all": true },
    { "op": "replace", "file": "src/b.py", "find": "old", "with": "new", "all": true },
    { "op": "append",  "file": "CHANGELOG.md", "text": "- renamed old to new" }
  ]
}"#,
            },
            Block::Prose(
                "This is the only way to edit more than one file in a single invocation. Each\n\
                 file is decoded, checked and reported on its own terms, so a batch spanning a\n\
                 UTF-8 file and a windows-1252 one is fine: each is written back in its own\n\
                 encoding.",
            ),
            Block::Prose(
                "Every operation runs against an in-memory copy and nothing reaches disk until\n\
                 all of them have succeeded, so a script that fails on its last operation leaves\n\
                 every file as it was. The writes themselves are then one atomic rename per\n\
                 file; intact cannot make a rename across several files atomic, so a disk error\n\
                 partway through that final step can leave earlier files written. A failing\n\
                 *operation* — no match, ambiguous anchor, unrepresentable character — never\n\
                 writes anything.",
            ),
            Block::Prose("Result reporting is per file, one summary line each:"),
            Block::Code {
                lang: "text",
                text: r#"src/a.py: updated (UTF-8, lf) - applied 1 operation(s)
src/b.py: updated (windows-1252, crlf) - applied 1 operation(s)"#,
            },
            Block::Prose(
                "With `--json`, batch reports a \"files\" array with one object per file — always\n\
                 an array, whether the script touched one file or twenty.",
            ),
        ],
    },
    Section {
        key: "recipes",
        title: "RECIPES",
        summary: "worked examples for common editing tasks",
        body: &[
            Block::Heading("INSPECT BEFORE EDITING AN UNFAMILIAR FILE"),
            Block::Code {
                lang: "bash",
                text: r#"intact info notes.txt
intact view notes.txt --lines 1:40 --number"#,
            },
            Block::Heading("CHANGE ONE UNIQUE LINE (the safe default)"),
            Block::Code {
                lang: "bash",
                text: r#"intact replace app.py --find 'timeout = 30' --with 'timeout = 60'"#,
            },
            Block::Prose("Exits 4 if that text appears more than once, 3 if it appears nowhere."),
            Block::Heading("RENAME A SYMBOL EVERYWHERE"),
            Block::Code {
                lang: "bash",
                text: r#"intact replace app.py --find old_name --with new_name --all
intact replace app.py --regex --all --find '\bold_name\b' --with new_name"#,
            },
            Block::Heading("DISAMBIGUATE A REPEATED STRING"),
            Block::Code {
                lang: "bash",
                text: r#"intact search app.py --find 'return None'          # see where they are
intact replace app.py --find 'return None' --with 'return []' --lines 40:80
intact replace app.py --find 'return None' --with 'return []' --occurrence 2"#,
            },
            Block::Heading("INSERT AN IMPORT AT THE TOP"),
            Block::Code {
                lang: "bash",
                text: r#"intact insert app.py --line 1 --text 'import os'"#,
            },
            Block::Heading("INSERT RELATIVE TO AN ANCHOR, WITHOUT KNOWING A LINE NUMBER"),
            Block::Prose(
                "Rewrite the anchor as itself plus the new line, rather than looking the line\n\
                 number up first:",
            ),
            Block::Code {
                lang: "bash",
                text: r#"intact replace app.py --escapes \
    --find 'import b' --with 'import b\nimport c'"#,
            },
            Block::Prose(
                "The same trick inserts before an anchor ('...\\nimport b'), and it keeps the\n\
                 edit anchored to content rather than to a position that may have moved.",
            ),
            Block::Heading("CREATE A FILE IN A DIRECTORY THAT DOES NOT EXIST YET"),
            Block::Code {
                lang: "bash",
                text: r#"intact create src/components/Foo.tsx --parents --text 'export const Foo = () => null;'"#,
            },
            Block::Prose(
                "Without `--parents` a missing directory is an error (exit 8) rather than a\n\
                 silently created tree. `create` refuses an existing file (exit 7); use `write`\n\
                 when replacing the contents is what you meant.",
            ),
            Block::Heading("SEVERAL CHANGES TO ONE FILE"),
            Block::Prose(
                "Do not run one command per change. Put them in a batch script, so they are\n\
                 applied in order and written once, and so a failure partway through leaves the\n\
                 file untouched rather than half-edited:",
            ),
            Block::Code {
                lang: "bash",
                text: r##"intact batch app.py --script - <<'EOF'
[{"op":"replace","find":"DEBUG = True","with":"DEBUG = False"},
 {"op":"replace","find":"log(","with":"logger.info(","all":true},
 {"op":"append","text":"# checked"}]
EOF"##,
            },
            Block::Prose(
                "This is also the answer to \"translate every comment in this file\" and similar\n\
                 sweeps: one script with one operation per comment.",
            ),
            Block::Heading("THE SAME CHANGE ACROSS SEVERAL FILES"),
            Block::Prose(
                "Give each operation its own \"file\". This is the only multi-file mode, and it is\n\
                 all-or-nothing across every file in the script:",
            ),
            Block::Code {
                lang: "bash",
                text: r#"intact batch --script - <<'EOF'
[{"op":"replace","file":"src/a.py","find":"old","with":"new","all":true},
 {"op":"replace","file":"src/b.py","find":"old","with":"new","all":true}]
EOF"#,
            },
            Block::Prose(
                "A shell loop still works when the file list comes from a glob, but note that a\n\
                 file where the text does not appear exits 3:",
            ),
            Block::Code {
                lang: "bash",
                text: r#"for f in src/*.py; do intact replace "$f" --find old --with new --all; done"#,
            },
            Block::Heading("REPLACE A BLOCK OF LINES WITH A FILE'S CONTENTS"),
            Block::Code {
                lang: "bash",
                text: r#"intact replace-lines app.py --lines 20:35 --text-file /tmp/new_block.py"#,
            },
            Block::Heading("INSERT MULTI-LINE TEXT IN ONE ARGUMENT"),
            Block::Code {
                lang: "bash",
                text: r#"intact insert app.py --line 1 --escapes --text 'import os\nimport sys'"#,
            },
            Block::Heading("DELETE A FUNCTION BODY"),
            Block::Code {
                lang: "bash",
                text: r#"intact delete app.py --lines 120:145"#,
            },
            Block::Heading("MOVE A BLOCK OF LINES"),
            Block::Code {
                lang: "bash",
                text: r#"intact move-lines app.py --lines 40:52 --after 12    # below line 12
intact move-lines app.py --lines 40:52 --before 1    # to the top
intact move-lines app.py --lines 40:52 --after $     # to the end
intact move-lines app.py --lines 7:9 --by -3         # up three lines"#,
            },
            Block::Prose(
                "`--after` and `--before` name a line of the file as `view --number` shows it\n\
                 now. `--by K` is the same move stated relatively: the block's first line ends\n\
                 up K lines further down, or further up for a negative K.",
            ),
            Block::Prose(
                "`move-lines` is addressed by line, like `delete` and `replace-lines`; there is\n\
                 no `--find` form. Find the block first, then move it:",
            ),
            Block::Code {
                lang: "bash",
                text: r#"intact search app.py --find 'def helper('   # app.py:40:1:def helper(x):
intact move-lines app.py --lines 40:52 --after 12"#,
            },
            Block::Heading("REGEX WITH CAPTURE GROUPS"),
            Block::Code {
                lang: "bash",
                text: r#"intact replace app.py --regex --all \
    --find 'def (\w+)\(' --with 'def test_$1('"#,
            },
            Block::Prose(
                "$1 and ${name} expand in the replacement; `--no-expand` disables that. Rust\n\
                 regex syntax; there are no backreferences or lookaround.",
            ),
            Block::Heading("SHOW WHAT THE EDIT DID"),
            Block::Code {
                lang: "bash",
                text: r#"intact --show-diff replace app.py --find x --with y --all"#,
            },
            Block::Heading("PREVIEW, THEN APPLY"),
            Block::Code {
                lang: "bash",
                text: r#"intact --dry-run replace app.py --find x --with y --all
intact replace app.py --find x --with y --all"#,
            },
            Block::Heading("SAVE THE CHANGE AS A PATCH"),
            Block::Code {
                lang: "bash",
                text: r#"intact --dry-run --quiet replace app.py --find x --with y > change.patch"#,
            },
            Block::Heading("EDIT A LEGACY-ENCODED FILE WITH A KNOWN ENCODING"),
            Block::Code {
                lang: "bash",
                text: r#"intact --encoding windows-1252 replace legacy.txt --find 'mundo' --with 'mundão'"#,
            },
            Block::Heading("MIGRATE A FILE TO UTF-8, THEN EDIT FREELY"),
            Block::Code {
                lang: "bash",
                text: r#"intact convert legacy.txt --to utf-8
intact replace legacy.txt --find 'mundo' --with '世界'"#,
            },
            Block::Heading("PASS TEXT THAT WILL NOT SURVIVE THE SHELL"),
            Block::Code {
                lang: "bash",
                text: r#"intact replace app.py --find 'old' --with-file /tmp/block.py
generate_block | intact replace app.py --find 'old' --with-file -"#,
            },
            Block::Heading("SCRIPTING AGAINST THE EXIT CODE"),
            Block::Code {
                lang: "bash",
                text: r#"intact replace app.py --find x --with y
case $? in
  0) echo done ;;
  3) echo 'not there' ;;
  4) echo 'ambiguous, narrow the search' ;;
  5) echo 'encoding problem, run intact info' ;;
esac"#,
            },
        ],
    },
];
