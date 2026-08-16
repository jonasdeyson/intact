//! The built-in manual, served by `intact guide [TOPIC]`.
//!
//! The executable is meant to be self-documenting: an agent handed nothing but
//! the binary must be able to discover the entire API starting from `--help`.
//! Everything the README says lives here too, so the two never drift apart in
//! the only direction that matters — the binary is the source of truth.

mod instructions;
mod sections;

pub use instructions::{InstructionsSpec, instructions};
pub use sections::SECTIONS;

pub struct Section {
    pub key: &'static str,
    pub title: &'static str,
    pub summary: &'static str,
    pub body: &'static str,
}

/// The encoding topic ends with the label list, which is generated from the
/// build's own table rather than restated by hand — that list used to be a
/// command of its own (`intact encodings`), which is one more thing to discover
/// for something nobody needs before they need the topic that explains it.
fn body(section: &Section) -> std::borrow::Cow<'static, str> {
    use std::borrow::Cow;
    if section.key != "encoding" {
        return Cow::Borrowed(section.body);
    }
    let mut out = String::from(section.body);
    out.push_str("\n\nEVERY LABEL THIS BUILD ACCEPTS\n\n");
    for label in crate::encoding_util::KNOWN_LABELS {
        out.push_str("  ");
        out.push_str(label);
        out.push('\n');
    }
    out.push_str(
        "\nAliases such as latin1, latin-1, iso-8859-1, cp1252 and ansi_x3.4-1968 are\n\
         accepted as well.",
    );
    Cow::Owned(out)
}

pub fn find(topic: &str) -> Option<&'static Section> {
    let needle = topic.trim().to_ascii_lowercase();
    SECTIONS.iter().find(|s| s.key == needle)
}

pub fn topic_list() -> String {
    let width = SECTIONS.iter().map(|s| s.key.len()).max().unwrap_or(0);
    let mut out = String::from("Manual topics (`intact guide TOPIC`):\n\n");
    for section in SECTIONS {
        out.push_str(&format!("  {:<width$}  {}\n", section.key, section.summary));
    }
    out.push_str("\n`intact guide` with no topic prints all of them.\n");
    out
}

/// Each heading carries the topic key, so a reader of the full manual knows how
/// to ask for that one section again.
fn heading(section: &Section) -> String {
    let line = format!("{}  (intact guide {})", section.title, section.key);
    format!("{line}\n{}\n", "-".repeat(line.len()))
}

pub fn render_all() -> String {
    let mut out = String::from("intact manual\n===============\n");
    for section in SECTIONS {
        out.push_str(&format!("\n\n{}\n", heading(section)));
        out.push_str(&body(section));
        out.push('\n');
    }
    out.push_str("\n\nSee also: `intact COMMAND --help` for a single command, and\n");
    out.push_str("`intact instructions` for a section to paste into a project's CLAUDE.md.\n");
    out
}

pub fn render_one(section: &Section) -> String {
    format!("{}\n{}\n", heading(section), body(section))
}
