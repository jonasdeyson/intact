//! The shape of the manual, and the two renderers that print it.
//!
//! A section body is a list of [`Block`]s rather than one blob of pre-formatted
//! text, so the same source can be printed as terminal text (`intact guide`)
//! and as Markdown (`intact guide --markdown`, which is what MANUAL.md is).
//! Neither output is authored by hand, so the two cannot drift.
//!
//! Prose is written as hard-wrapped lines with inline Markdown — backticks and
//! `*emphasis*` — allowed and passed through untouched by both renderers.
//! Anything structural (a heading, a list, a table, a code sample) is a block,
//! never something a renderer has to recognise by looking at the text.

/// One piece of a section body.
///
/// Every variant holds `&'static` data so that [`super::SECTIONS`] stays a
/// `const` with nothing built at run time.
pub enum Block {
    /// A heading below the section title. Sections are flat: there is no
    /// deeper level, and a section that seems to want one wants splitting.
    Heading(&'static str),

    /// A paragraph. Hard-wrapped in the source; Markdown reflows it.
    Prose(&'static str),

    /// A literal sample — a command, a JSON script, program output. `lang`
    /// labels the Markdown fence (`bash`, `json`, `text`) and is ignored by
    /// the plain renderer, which indents instead.
    Code {
        lang: &'static str,
        text: &'static str,
    },

    /// A bullet list. Items may be hard-wrapped; continuation lines are
    /// re-indented under the item text.
    Bullets(&'static [&'static str]),

    /// A numbered list, numbered from 1 by the renderer.
    Numbered(&'static [&'static str]),

    /// A table. An empty `head` means the columns are self-evident: the plain
    /// renderer omits the header row and the Markdown one leaves it blank.
    /// An empty `caption` means no caption.
    Table {
        caption: &'static str,
        head: &'static [&'static str],
        rows: &'static [&'static [&'static str]],
    },

    /// A flowed list of short strings in however many columns fit — the
    /// encoding label list, which comes from the build's own table.
    Labels(&'static [&'static str]),
}

/// The width both renderers wrap generated text to. Authored prose is wrapped
/// in the source instead, so this only governs tables and label lists.
const WIDTH: usize = 79;

// ------------------------------------------------------------------- plain

/// Render blocks as the terminal text `intact guide` prints.
pub fn plain(blocks: &[Block]) -> String {
    let mut out = String::new();
    for (i, block) in blocks.iter().enumerate() {
        if !out.is_empty() {
            out.push('\n');
        }
        match block {
            Block::Heading(text) => {
                out.push_str(text);
                out.push('\n');
            }
            Block::Prose(text) => {
                out.push_str(text);
                out.push('\n');
            }
            Block::Code { text, .. } => {
                for line in text.lines() {
                    if line.is_empty() {
                        out.push('\n');
                    } else {
                        out.push_str("  ");
                        out.push_str(line);
                        out.push('\n');
                    }
                }
            }
            Block::Bullets(items) => push_list(&mut out, items, None),
            Block::Numbered(items) => push_list(&mut out, items, Some(())),
            Block::Table {
                caption,
                head,
                rows,
            } => plain_table(&mut out, caption, head, rows, &run_widths(blocks, i)),
            Block::Labels(labels) => {
                for line in flow(labels, WIDTH - 2) {
                    out.push_str("  ");
                    out.push_str(&line);
                    out.push('\n');
                }
            }
        }
    }
    out
}

/// Render a list. `numbered` picks the marker; both renderings are the same
/// here, since a hanging indent is valid Markdown and reads correctly as text.
///
/// A list whose items run to several lines is spaced out, the way Markdown
/// itself distinguishes a loose list from a tight one: without the blank lines
/// a five-item list of paragraphs is a wall, and Markdown would reflow the
/// items into one paragraph each anyway.
fn push_list(out: &mut String, items: &[&str], numbered: Option<()>) {
    let loose = items.iter().any(|i| i.contains('\n'));
    for (n, item) in items.iter().enumerate() {
        if loose && n > 0 {
            out.push('\n');
        }
        let marker = match numbered {
            Some(()) => format!("{}. ", n + 1),
            None => "- ".to_string(),
        };
        let pad = " ".repeat(marker.len());
        for (i, line) in item.lines().enumerate() {
            out.push_str(if i == 0 { &marker } else { &pad });
            out.push_str(line);
            out.push('\n');
        }
    }
}

/// Column widths for the table at `at`, measured across every table in the run
/// of adjacent tables it belongs to.
///
/// Splitting a grouped list into one table per group is how the grouping gets
/// said out loud, but a reader sees the groups as one list and a column that
/// jumps width between them looks broken. Markdown has no such problem — each
/// table is laid out by the renderer — so this lives on the plain side only.
fn run_widths(blocks: &[Block], at: usize) -> Vec<usize> {
    let is_table = |b: &Block| matches!(b, Block::Table { .. });
    let mut start = at;
    while start > 0 && is_table(&blocks[start - 1]) {
        start -= 1;
    }
    let mut end = at + 1;
    while end < blocks.len() && is_table(&blocks[end]) {
        end += 1;
    }

    let mut widths: Vec<usize> = Vec::new();
    for block in &blocks[start..end] {
        if let Block::Table { head, rows, .. } = block {
            for (i, w) in column_widths(head, rows).into_iter().enumerate() {
                match widths.get_mut(i) {
                    Some(current) => *current = (*current).max(w),
                    None => widths.push(w),
                }
            }
        }
    }
    widths
}

fn plain_table(out: &mut String, caption: &str, head: &[&str], rows: &[&[&str]], widths: &[usize]) {
    let indent = if caption.is_empty() { 2 } else { 4 };
    if !caption.is_empty() {
        out.push_str("  ");
        out.push_str(caption);
        out.push_str(":\n");
    }

    // Everything but the last column is padded to its width; the last one gets
    // whatever is left of the line and wraps into it.
    let fixed: usize = widths.iter().rev().skip(1).map(|w| w + 2).sum();
    let last = WIDTH.saturating_sub(indent + fixed).max(20);

    if !head.is_empty() {
        push_row(out, indent, widths, last, head);
        // The final column's width is what its widest cell wants, which may be
        // more than the line has left; that cell wraps, so its rule must be
        // drawn to the width actually used rather than the width asked for.
        let mut rule: Vec<String> = widths.iter().map(|w| "-".repeat(*w)).collect();
        if let Some(tail) = rule.last_mut() {
            tail.truncate(tail.len().min(last));
        }
        let rule: Vec<&str> = rule.iter().map(String::as_str).collect();
        push_row(out, indent, widths, last, &rule);
    }
    for row in rows {
        push_row(out, indent, widths, last, row);
    }
}

fn push_row(out: &mut String, indent: usize, widths: &[usize], last: usize, cells: &[&str]) {
    let split = cells.len().saturating_sub(1);
    let mut lead = " ".repeat(indent);
    for (i, cell) in cells.iter().take(split).enumerate() {
        lead.push_str(cell);
        lead.push_str(&" ".repeat(widths[i] - chars(cell) + 2));
    }
    let tail = cells.last().copied().unwrap_or("");
    let hang = " ".repeat(chars(&lead));
    for (i, line) in wrap(tail, last).iter().enumerate() {
        out.push_str(if i == 0 { &lead } else { &hang });
        out.push_str(line);
        // Trailing padding on an empty cell would be invisible whitespace.
        while out.ends_with(' ') {
            out.pop();
        }
        out.push('\n');
    }
}

// ---------------------------------------------------------------- markdown

/// Render blocks as Markdown. `level` is the heading level a [`Block::Heading`]
/// gets, so a section printed on its own and the same section inside MANUAL.md
/// sit at the right depth without the source knowing where it will land.
pub fn markdown(blocks: &[Block], level: u8) -> String {
    let hashes = "#".repeat(level.clamp(1, 6) as usize);
    let mut out = String::new();
    for block in blocks {
        if !out.is_empty() {
            out.push('\n');
        }
        match block {
            Block::Heading(text) => {
                out.push_str(&format!("{hashes} {}\n", sentence_case(text)));
            }
            Block::Prose(text) => {
                out.push_str(text);
                out.push('\n');
            }
            Block::Code { lang, text } => {
                out.push_str(&format!("```{lang}\n{}\n```\n", text.trim_end()));
            }
            Block::Bullets(items) => push_list(&mut out, items, None),
            Block::Numbered(items) => push_list(&mut out, items, Some(())),
            Block::Table {
                caption,
                head,
                rows,
            } => md_table(&mut out, caption, head, rows),
            Block::Labels(labels) => {
                out.push_str("```text\n");
                for line in flow(labels, WIDTH) {
                    out.push_str(&line);
                    out.push('\n');
                }
                out.push_str("```\n");
            }
        }
    }
    out
}

fn md_table(out: &mut String, caption: &str, head: &[&str], rows: &[&[&str]]) {
    if !caption.is_empty() {
        out.push_str(&format!("**{caption}**\n\n"));
    }
    let columns = rows.iter().map(|r| r.len()).max().unwrap_or(head.len());
    let header: Vec<String> = (0..columns)
        .map(|i| head.get(i).copied().unwrap_or("").to_string())
        .collect();
    out.push_str(&md_row(&header));
    out.push_str(&format!("|{}\n", " --- |".repeat(columns)));
    for row in rows {
        let cells: Vec<String> = (0..columns)
            .map(|i| row.get(i).copied().unwrap_or("").to_string())
            .collect();
        out.push_str(&md_row(&cells));
    }
}

fn md_row(cells: &[String]) -> String {
    let mut line = String::from("|");
    for cell in cells {
        // A pipe inside a cell would end it; a hard-wrapped cell would end the
        // row. Neither is worth rejecting at compile time, so fix both here.
        line.push(' ');
        line.push_str(&cell.replace('|', r"\|").replace('\n', " "));
        line.push_str(" |");
    }
    line.push('\n');
    line
}

// ----------------------------------------------------------------- helpers

fn chars(s: &str) -> usize {
    s.chars().count()
}

/// Headings are written in capitals, which is how a heading announces itself in
/// a terminal that has no other way to say it. A Markdown `#` carries its own
/// weight, so the capitals come off there — as sentence case rather than title
/// case, because several headings are whole sentences and Title Case Reads
/// Like This. Words that are genuinely spelled in capitals keep them.
pub fn sentence_case(text: &str) -> String {
    const ACRONYMS: &[&str] = &["JSON", "BOM", "BOMS", "UTF-8", "UTF-16", "CLI", "I/O", "AI"];
    text.split(' ')
        .enumerate()
        .map(|(i, word)| {
            if ACRONYMS.contains(&word) {
                return word.to_string();
            }
            let lower = word.to_lowercase();
            if i > 0 {
                return lower;
            }
            let mut c = lower.chars();
            match c.next() {
                Some(first) => first.to_uppercase().to_string() + c.as_str(),
                None => String::new(),
            }
        })
        .collect::<Vec<_>>()
        .join(" ")
}

fn column_widths(head: &[&str], rows: &[&[&str]]) -> Vec<usize> {
    let columns = rows.iter().map(|r| r.len()).max().unwrap_or(head.len());
    (0..columns)
        .map(|i| {
            let in_head = head.get(i).map(|c| chars(c)).unwrap_or(0);
            rows.iter()
                .filter_map(|r| r.get(i))
                .map(|c| chars(c))
                .chain(std::iter::once(in_head))
                .max()
                .unwrap_or(0)
        })
        .collect()
}

/// Greedy word wrap. Only table cells and label lists need it — authored prose
/// is wrapped in the source, where the author can see the result.
fn wrap(text: &str, width: usize) -> Vec<String> {
    let mut lines = Vec::new();
    let mut current = String::new();
    for word in text.split_whitespace() {
        if current.is_empty() {
            current.push_str(word);
        } else if chars(&current) + 1 + chars(word) <= width {
            current.push(' ');
            current.push_str(word);
        } else {
            lines.push(std::mem::take(&mut current));
            current.push_str(word);
        }
    }
    lines.push(current);
    lines
}

/// Lay short strings out in as many equal columns as fit.
fn flow(items: &[&str], width: usize) -> Vec<String> {
    let cell = items.iter().map(|s| chars(s)).max().unwrap_or(0) + 2;
    let columns = (width / cell).max(1);
    items
        .chunks(columns)
        .map(|row| {
            let mut line = String::new();
            for item in row {
                line.push_str(item);
                line.push_str(&" ".repeat(cell - chars(item)));
            }
            while line.ends_with(' ') {
                line.pop();
            }
            line
        })
        .collect()
}
