//! A deliberately small line diff, used only for `--dry-run` previews.
//!
//! Common prefix and suffix lines are trimmed and whatever remains is shown as
//! a removed block followed by an added block. That is exact about *what*
//! changed without pulling in a diff library.

pub struct Hunk {
    pub old_start: usize,
    pub removed: Vec<String>,
    pub new_start: usize,
    pub added: Vec<String>,
}

fn split(text: &str) -> Vec<String> {
    crate::lines::LineIndex::build(text)
        .lines
        .iter()
        .map(|l| text[l.start..l.content_end].to_string())
        .collect()
}

pub fn diff(before: &str, after: &str) -> Option<Hunk> {
    let a = split(before);
    let b = split(after);

    let mut prefix = 0;
    while prefix < a.len() && prefix < b.len() && a[prefix] == b[prefix] {
        prefix += 1;
    }
    let mut suffix = 0;
    while suffix < a.len() - prefix
        && suffix < b.len() - prefix
        && a[a.len() - 1 - suffix] == b[b.len() - 1 - suffix]
    {
        suffix += 1;
    }

    let removed: Vec<String> = a[prefix..a.len() - suffix].to_vec();
    let added: Vec<String> = b[prefix..b.len() - suffix].to_vec();
    if removed.is_empty() && added.is_empty() {
        return None;
    }
    Some(Hunk {
        old_start: prefix + 1,
        removed,
        new_start: prefix + 1,
        added,
    })
}

/// Render a hunk, capping the number of lines shown.
pub fn render(hunk: &Hunk, max_lines: usize) -> String {
    let mut out = String::new();
    out.push_str(&format!(
        "@@ -{},{} +{},{} @@\n",
        hunk.old_start,
        hunk.removed.len(),
        hunk.new_start,
        hunk.added.len()
    ));
    for (i, line) in hunk.removed.iter().enumerate() {
        if i == max_lines {
            out.push_str(&format!(
                "  ... {} more removed line(s)\n",
                hunk.removed.len() - i
            ));
            break;
        }
        out.push_str(&format!("-{line}\n"));
    }
    for (i, line) in hunk.added.iter().enumerate() {
        if i == max_lines {
            out.push_str(&format!(
                "  ... {} more added line(s)\n",
                hunk.added.len() - i
            ));
            break;
        }
        out.push_str(&format!("+{line}\n"));
    }
    out
}
