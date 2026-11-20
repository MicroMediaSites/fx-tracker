//! Plain-language playbooks (`~/.wickd/playbooks/<name>.md`, wickd-learn
//! D-20, AGT-2546).
//!
//! A playbook is a markdown strategy doc the user drops in, written for a
//! person, not a rule engine. The agent looks one up when a conversation
//! needs it (`wickd playbook search …` returns the matching section) and
//! reads it as loose guidance; playbooks are never attached to every prompt.
//!
//! Each playbook is one file named `<name>.md`, where the name follows the
//! list-name rule (`a-z`, `0-9`, `-`, `_`, max 32). Writes are a tmp file +
//! rename, so a reader never sees a half-written playbook. The store honours
//! `WICKD_HOME` like every other wickd store.

use std::path::{Path, PathBuf};

use anyhow::{anyhow, Context, Result};
use serde::Serialize;

use crate::lists::invalid;

/// The most sections `search` returns per playbook (the rest are counted).
pub const MAX_SECTIONS: usize = 5;

/// The playbooks directory under the wickd data home.
pub fn playbooks_dir() -> Result<PathBuf> {
    Ok(wickd_core::paths::wickd_data_home()
        .map_err(|e| anyhow!(e))?
        .join("playbooks"))
}

/// Playbook names: the list-name rule (they're typed in commands).
pub fn validate_name(name: &str) -> Result<String> {
    let n = name.trim().to_ascii_lowercase();
    if n.is_empty()
        || n.len() > 32
        || !n
            .chars()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-' || c == '_')
    {
        return Err(invalid(format!(
            "invalid playbook name '{name}': use a-z, 0-9, - or _ (max 32)"
        )));
    }
    Ok(n)
}

/// The default name for a file: its stem, lower-cased, with every run of
/// other characters turned into one `-` (`My Strategy.md` → `my-strategy`).
/// Still checked by [`validate_name`] (e.g. a stem over 32 characters fails
/// and asks for `--name`).
pub fn name_from_path(path: &Path) -> Result<String> {
    let stem = path
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or_default();
    let mut slug = String::new();
    for c in stem.trim().to_ascii_lowercase().chars() {
        if c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-' || c == '_' {
            slug.push(c);
        } else if !slug.ends_with('-') {
            slug.push('-');
        }
    }
    let slug = slug.trim_matches('-');
    validate_name(slug).map_err(|_| {
        invalid(format!(
            "can't make a playbook name from '{}': give one with --name (a-z, 0-9, - or _, max 32)",
            path.display()
        ))
    })
}

/// The error for a playbook that doesn't exist (with the next step).
pub fn no_playbook(name: &str) -> anyhow::Error {
    invalid(format!("no playbook '{name}' (see `wickd playbook ls`)"))
}

fn file_for(dir: &Path, name: &str) -> PathBuf {
    dir.join(format!("{name}.md"))
}

/// The title: the first `# ` heading (outside code fences), else the name.
pub fn title(name: &str, text: &str) -> String {
    let mut fence = false;
    for line in text.lines() {
        let t = line.trim_start();
        if is_fence(t) {
            fence = !fence;
            continue;
        }
        if fence {
            continue;
        }
        if let Some(h) = t.strip_prefix("# ") {
            let h = h.trim().trim_end_matches('#').trim();
            if !h.is_empty() {
                return h.to_string();
            }
        }
    }
    name.to_string()
}

fn is_fence(trimmed: &str) -> bool {
    trimmed.starts_with("```") || trimmed.starts_with("~~~")
}

/// A heading line's text (`## 5.1 Signal` → `5.1 Signal`), if it is one.
fn heading(trimmed: &str) -> Option<&str> {
    let hashes = trimmed.chars().take_while(|c| *c == '#').count();
    if hashes == 0 || hashes > 6 {
        return None;
    }
    let rest = &trimmed[hashes..];
    if !rest.is_empty() && !rest.starts_with(' ') && !rest.starts_with('\t') {
        return None;
    }
    Some(rest.trim().trim_end_matches('#').trim())
}

/// One playbook in `ls`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Summary {
    pub name: String,
    pub title: String,
    /// Bytes.
    pub size: u64,
}

/// Copy `text` into the store as `name` (replacing one of the same name).
/// Returns whether it replaced an existing playbook.
pub fn add_at(dir: &Path, name: &str, text: &str) -> Result<bool> {
    std::fs::create_dir_all(dir).with_context(|| format!("creating {}", dir.display()))?;
    let path = file_for(dir, name);
    let replaced = path.exists();
    // Per-process tmp name, so concurrent adds never share one.
    let tmp = dir.join(format!(".{name}.md.{}.tmp", std::process::id()));
    std::fs::write(&tmp, text).with_context(|| format!("writing {}", tmp.display()))?;
    std::fs::rename(&tmp, &path).with_context(|| format!("replacing {}", path.display()))?;
    Ok(replaced)
}

/// Read a source file for `add`. A missing or unreadable file, or one that
/// isn't text, is the caller's mistake (a validation error).
pub fn read_source(path: &Path) -> Result<String> {
    let bytes = std::fs::read(path).map_err(|e| invalid(format!("{}: {e}", path.display())))?;
    String::from_utf8(bytes).map_err(|_| invalid(format!("{} is not UTF-8 text", path.display())))
}

/// Every playbook, by name (an absent store is empty).
pub fn list_at(dir: &Path) -> Result<Vec<Summary>> {
    let rd = match std::fs::read_dir(dir) {
        Ok(rd) => rd,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(e) => return Err(e).with_context(|| format!("reading {}", dir.display())),
    };
    let mut out = Vec::new();
    for entry in rd {
        let entry = entry.with_context(|| format!("reading {}", dir.display()))?;
        let path = entry.path();
        if path.extension().and_then(|e| e.to_str()) != Some("md") {
            continue;
        }
        let Some(name) = path.file_stem().and_then(|s| s.to_str()) else {
            continue;
        };
        // Skip anything that isn't a playbook name (e.g. a stray tmp file).
        if validate_name(name).ok().as_deref() != Some(name) {
            continue;
        }
        let text = std::fs::read_to_string(&path)
            .with_context(|| format!("reading {}", path.display()))?;
        out.push(Summary {
            name: name.to_string(),
            title: title(name, &text),
            size: text.len() as u64,
        });
    }
    out.sort_by(|a, b| a.name.cmp(&b.name));
    Ok(out)
}

/// One playbook's full text.
pub fn show_at(dir: &Path, name: &str) -> Result<String> {
    let path = file_for(dir, name);
    match std::fs::read_to_string(&path) {
        Ok(t) => Ok(t),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Err(no_playbook(name)),
        Err(e) => Err(e).with_context(|| format!("reading {}", path.display())),
    }
}

/// Remove a playbook.
pub fn remove_at(dir: &Path, name: &str) -> Result<()> {
    let path = file_for(dir, name);
    match std::fs::remove_file(&path) {
        Ok(()) => Ok(()),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Err(no_playbook(name)),
        Err(e) => Err(e).with_context(|| format!("removing {}", path.display())),
    }
}

/// A matching section: the nearest heading above and its paragraph.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Section {
    /// The nearest heading above (empty before the first heading).
    pub heading: String,
    pub text: String,
}

/// A playbook with at least one matching section.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Hit {
    pub name: String,
    pub title: String,
    /// How many sections matched (`sections` holds the first few).
    pub matches: usize,
    pub sections: Vec<Section>,
}

/// The sections of `text` that contain every word (case-insensitive). A
/// paragraph matches on its own text; a heading whose own text matches but
/// none of whose paragraphs do returns its first paragraph. The best
/// matches come first (the words as a phrase, then under a heading that
/// names them), otherwise document order.
pub fn matching_sections(text: &str, words: &[String]) -> Vec<Section> {
    let words: Vec<String> = words
        .iter()
        .map(|w| w.trim().to_lowercase())
        .filter(|w| !w.is_empty())
        .collect();
    if words.is_empty() {
        return Vec::new();
    }
    let has_all = |s: &str| {
        let s = s.to_lowercase();
        words.iter().all(|w| s.contains(w.as_str()))
    };
    let mut out = Vec::new();
    for (head, paras) in split_sections(text) {
        let before = out.len();
        for p in &paras {
            if has_all(p) {
                out.push(Section {
                    heading: head.clone(),
                    text: p.clone(),
                });
            }
        }
        if out.len() == before && !head.is_empty() && has_all(&head) {
            out.push(Section {
                heading: head.clone(),
                text: paras.first().cloned().unwrap_or_default(),
            });
        }
    }
    let phrase = words.join(" ");
    let score = |s: &Section| {
        let in_text = s.text.to_lowercase().contains(&phrase) as u8;
        let in_head = has_all(&s.heading) as u8;
        2 * in_text + in_head
    };
    // Stable: equal scores keep document order.
    out.sort_by_key(|s| std::cmp::Reverse(score(s)));
    out
}

/// Split markdown into (heading, paragraphs) groups. Paragraphs are runs of
/// non-blank lines; code fences are kept whole and never read as headings.
fn split_sections(text: &str) -> Vec<(String, Vec<String>)> {
    let mut groups: Vec<(String, Vec<String>)> = Vec::new();
    // The group being filled: its heading and paragraphs.
    let mut head = String::new();
    let mut paras: Vec<String> = Vec::new();
    let mut para: Vec<&str> = Vec::new();
    let mut fence = false;
    fn flush(paras: &mut Vec<String>, para: &mut Vec<&str>) {
        let p = para.join("\n").trim().to_string();
        if !p.is_empty() {
            paras.push(p);
        }
        para.clear();
    }
    for line in text.lines() {
        let t = line.trim_start();
        if is_fence(t) {
            fence = !fence;
            para.push(line);
            continue;
        }
        if fence {
            para.push(line);
            continue;
        }
        if let Some(h) = heading(t) {
            flush(&mut paras, &mut para);
            let prev = std::mem::replace(&mut head, h.to_string());
            groups.push((prev, std::mem::take(&mut paras)));
        } else if t.is_empty() {
            flush(&mut paras, &mut para);
        } else {
            para.push(line);
        }
    }
    flush(&mut paras, &mut para);
    groups.push((head, paras));
    groups
        .into_iter()
        .filter(|(h, p)| !h.is_empty() || !p.is_empty())
        .collect()
}

/// Search every playbook for `words`: those with a match, most matches first.
pub fn search_at(dir: &Path, words: &[String]) -> Result<Vec<Hit>> {
    let mut hits = Vec::new();
    for s in list_at(dir)? {
        let text = show_at(dir, &s.name)?;
        let mut sections = matching_sections(&text, words);
        if sections.is_empty() {
            continue;
        }
        let matches = sections.len();
        sections.truncate(MAX_SECTIONS);
        hits.push(Hit {
            name: s.name,
            title: s.title,
            matches,
            sections,
        });
    }
    hits.sort_by(|a, b| b.matches.cmp(&a.matches).then(a.name.cmp(&b.name)));
    Ok(hits)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::lists::ValidationError;

    const DOC: &str = "\
# RaHaG0D Ichimoku

Intro paragraph.

## 5.1 Signal Type 1 — Tenkan Close + Range Break

**Step A — Valid Tenkan close:**
the candle CLOSES beyond the Tenkan-sen.

Breakout within 9 bars.

```sh
# not a heading: tenkan close in a fence
```

## 6. Filters

Nothing about it here.
";

    fn is_validation(e: &anyhow::Error) -> bool {
        e.downcast_ref::<ValidationError>().is_some()
    }

    #[test]
    fn add_replaces_and_ls_shows_titles() {
        let dir = tempfile::tempdir().unwrap();
        let d = dir.path().join("playbooks");
        // An empty (absent) store lists as empty, not an error.
        assert!(list_at(&d).unwrap().is_empty());
        assert!(!add_at(&d, "rahagod", DOC).unwrap());
        assert!(!add_at(&d, "plain", "no heading here\n").unwrap());
        let ls = list_at(&d).unwrap();
        assert_eq!(
            ls,
            vec![
                Summary {
                    name: "plain".into(),
                    title: "plain".into(),
                    size: 16
                },
                Summary {
                    name: "rahagod".into(),
                    title: "RaHaG0D Ichimoku".into(),
                    size: DOC.len() as u64
                },
            ]
        );
        // Re-adding a name replaces it.
        assert!(add_at(&d, "plain", "# Replaced\n").unwrap());
        assert_eq!(show_at(&d, "plain").unwrap(), "# Replaced\n");
        assert_eq!(list_at(&d).unwrap()[0].title, "Replaced");
        remove_at(&d, "plain").unwrap();
        assert_eq!(list_at(&d).unwrap().len(), 1);
    }

    #[test]
    fn title_skips_fences_and_deeper_headings() {
        assert_eq!(title("x", "```\n# code\n```\n## Two\n# One #\n"), "One");
        assert_eq!(title("x", "#nospace\n"), "x");
    }

    #[test]
    fn search_returns_the_heading_and_its_paragraph() {
        let words = vec!["TENKAN".to_string(), "close".to_string()];
        let s = matching_sections(DOC, &words);
        assert_eq!(
            s,
            vec![
                Section {
                    heading: "5.1 Signal Type 1 — Tenkan Close + Range Break".into(),
                    text:
                        "**Step A — Valid Tenkan close:**\nthe candle CLOSES beyond the Tenkan-sen."
                            .into(),
                },
                Section {
                    heading: "5.1 Signal Type 1 — Tenkan Close + Range Break".into(),
                    text: "```sh\n# not a heading: tenkan close in a fence\n```".into(),
                }
            ]
        );
        // A heading-only match returns its first paragraph.
        let s = matching_sections(DOC, &["filters".to_string()]);
        assert_eq!(s.len(), 1);
        assert_eq!(
            (s[0].heading.as_str(), s[0].text.as_str()),
            ("6. Filters", "Nothing about it here.")
        );
        // Every word must be in the section.
        assert!(matching_sections(DOC, &["tenkan".into(), "filters".into()]).is_empty());
        assert!(matching_sections(DOC, &[" ".into()]).is_empty());
        // The phrase under a heading naming it ranks above a scattered match.
        let ranked = "## A\n\nclose to the tenkan\n\n## Tenkan close\n\nthe tenkan close rule\n";
        let texts: Vec<_> = matching_sections(ranked, &words)
            .into_iter()
            .map(|s| s.text)
            .collect();
        assert_eq!(texts, ["the tenkan close rule", "close to the tenkan"]);

        let dir = tempfile::tempdir().unwrap();
        add_at(dir.path(), "rahagod", DOC).unwrap();
        add_at(dir.path(), "other", "# Other\n\nbreakout only\n").unwrap();
        let hits = search_at(dir.path(), &["breakout".into()]).unwrap();
        let names: Vec<_> = hits.iter().map(|h| h.name.as_str()).collect();
        assert_eq!(names, ["other", "rahagod"]);
        assert_eq!(hits[1].sections[0].text, "Breakout within 9 bars.");
        assert!(search_at(dir.path(), &["zzz".into()]).unwrap().is_empty());
    }

    #[test]
    fn names_are_normalised_and_errors_are_validation() {
        assert_eq!(validate_name(" RaHaGoD ").unwrap(), "rahagod");
        for bad in ["", "my playbook", "../x", &"a".repeat(33)] {
            assert!(is_validation(&validate_name(bad).unwrap_err()), "{bad}");
        }
        assert_eq!(
            name_from_path(Path::new("/x/My Strategy (v2).md")).unwrap(),
            "my-strategy-v2"
        );
        assert_eq!(
            name_from_path(Path::new("rahagod-ichimoku-encoding-spec.md")).unwrap(),
            "rahagod-ichimoku-encoding-spec"
        );
        assert!(is_validation(
            &name_from_path(Path::new("/x/!!!.md")).unwrap_err()
        ));
        assert!(is_validation(
            &name_from_path(&PathBuf::from(format!("{}.md", "a".repeat(40)))).unwrap_err()
        ));

        let dir = tempfile::tempdir().unwrap();
        assert!(is_validation(&show_at(dir.path(), "nope").unwrap_err()));
        assert!(is_validation(&remove_at(dir.path(), "nope").unwrap_err()));
        assert!(is_validation(
            &read_source(&dir.path().join("missing.md")).unwrap_err()
        ));
        let bin = dir.path().join("bin.md");
        std::fs::write(&bin, [0xff, 0xfe, 0x00]).unwrap();
        assert!(is_validation(&read_source(&bin).unwrap_err()));
    }
}
