//! The journal screen's state: the markdown files of a journal folder, one
//! open at a time.
//!
//! The folder is `WICKD_JOURNAL_DIR` if set, else `~/trading-journal`. Files
//! are read where they are (nothing is copied under `~/.wickd`), so an entry
//! being written by an editor or an agent shows up as it changes: the event
//! loop calls [`JournalState::poll`] every couple of seconds while the screen
//! is open.

use std::cell::{Cell, RefCell};
use std::ffi::OsString;
use std::path::{Path, PathBuf};
use std::time::SystemTime;

use super::app::Mode;
use super::markdown::{self, Rendered};

/// Where the journal lives: `WICKD_JOURNAL_DIR`, else `~/trading-journal`.
pub fn default_dir() -> PathBuf {
    dir_from(
        std::env::var_os("WICKD_JOURNAL_DIR"),
        std::env::var_os("HOME"),
    )
}

/// [`default_dir`] with its inputs passed in (testable without touching the
/// process environment).
pub fn dir_from(env: Option<OsString>, home: Option<OsString>) -> PathBuf {
    match env.filter(|v| !v.is_empty()) {
        Some(d) => PathBuf::from(d),
        None => PathBuf::from(home.unwrap_or_default()).join("trading-journal"),
    }
}

/// One journal file.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Entry {
    /// File name without `.md` (`2026-11-13`).
    pub name: String,
    pub path: PathBuf,
}

/// The `*.md` files directly inside `dir`, sorted by name (date-named files
/// therefore run oldest → newest).
pub fn list_entries(dir: &Path) -> std::io::Result<Vec<Entry>> {
    let mut out: Vec<Entry> = std::fs::read_dir(dir)?
        .filter_map(Result::ok)
        .map(|e| e.path())
        .filter(|p| p.is_file() && p.extension().is_some_and(|x| x.eq_ignore_ascii_case("md")))
        .filter_map(|p| {
            let name = p.file_stem()?.to_str()?.to_string();
            (!name.starts_with('.')).then_some(Entry { name, path: p })
        })
        .collect();
    out.sort_by(|a, b| a.name.cmp(&b.name));
    Ok(out)
}

#[derive(Debug)]
pub struct JournalState {
    pub dir: PathBuf,
    pub entries: Vec<Entry>,
    /// The open entry (index into `entries`).
    pub idx: usize,
    /// The open entry's text.
    pub text: String,
    /// The open file's modification time when it was read.
    pub mtime: Option<SystemTime>,
    /// First line shown.
    pub scroll: usize,
    /// Why the folder or file couldn't be read.
    pub error: Option<String>,
    /// Where `esc` returns to.
    pub back_to: Mode,
    /// Body rows on screen, as last measured by the renderer.
    pub page: Cell<usize>,
    /// The laid-out text for the width it was last drawn at.
    cache: RefCell<Option<(u16, Rendered)>>,
}

impl JournalState {
    pub fn new(dir: PathBuf) -> Self {
        Self {
            dir,
            entries: Vec::new(),
            idx: 0,
            text: String::new(),
            mtime: None,
            scroll: 0,
            error: None,
            back_to: Mode::Watchlist,
            page: Cell::new(20),
            cache: RefCell::new(None),
        }
    }

    pub fn current(&self) -> Option<&Entry> {
        self.entries.get(self.idx)
    }

    /// Open the screen's content: list the folder and show the newest entry.
    pub fn open(&mut self) {
        self.relist(None);
        self.idx = self.entries.len().saturating_sub(1);
        self.scroll = 0;
        self.read();
    }

    /// Re-read the folder and the open file, staying on the same entry and
    /// scroll position (`r`).
    pub fn reload(&mut self) {
        let keep = self.current().map(|e| e.name.clone());
        self.relist(keep.as_deref());
        self.read();
    }

    /// Pick up changes made outside the TUI: new or removed files, and edits
    /// to the open one (by modification time). Returns whether anything
    /// changed.
    pub fn poll(&mut self) -> bool {
        let before = (self.entries.clone(), self.mtime, self.error.clone());
        let keep = self.current().map(|e| e.name.clone());
        self.relist(keep.as_deref());
        let moved = self.current().map(|e| &e.name) != keep.as_ref();
        if moved || self.current().and_then(|e| file_mtime(&e.path)) != self.mtime {
            self.read();
        }
        before != (self.entries.clone(), self.mtime, self.error.clone())
    }

    /// List the folder; stay on the entry called `keep` when it's still there.
    fn relist(&mut self, keep: Option<&str>) {
        match list_entries(&self.dir) {
            Ok(e) => {
                self.entries = e;
                self.error = None;
            }
            Err(e) => {
                self.entries.clear();
                // A folder that isn't there is the empty state, not an error.
                self.error = (e.kind() != std::io::ErrorKind::NotFound)
                    .then(|| format!("{}: {e}", self.dir.display()));
            }
        }
        self.idx = keep
            .and_then(|k| self.entries.iter().position(|e| e.name == k))
            .unwrap_or_else(|| self.idx.min(self.entries.len().saturating_sub(1)));
    }

    /// Read the open entry from disk.
    fn read(&mut self) {
        self.cache.replace(None);
        let Some(e) = self.entries.get(self.idx) else {
            self.text.clear();
            self.mtime = None;
            return;
        };
        match std::fs::read(&e.path) {
            Ok(bytes) => {
                self.text = String::from_utf8_lossy(&bytes).into_owned();
                self.mtime = file_mtime(&e.path);
            }
            Err(err) => {
                self.text.clear();
                self.mtime = None;
                self.error = Some(format!("{}: {err}", e.path.display()));
            }
        }
    }

    /// Move `delta` entries (negative = older); false at either end.
    pub fn step_entry(&mut self, delta: isize) -> bool {
        let n = self.entries.len() as isize;
        let to = self.idx as isize + delta;
        if n == 0 || to < 0 || to >= n {
            return false;
        }
        self.idx = to as usize;
        self.scroll = 0;
        self.read();
        true
    }

    /// The text laid out for `width` (cached until the text or width changes).
    pub fn with_rendered<T>(&self, width: u16, f: impl FnOnce(&Rendered) -> T) -> T {
        let mut cache = self.cache.borrow_mut();
        if cache.as_ref().is_none_or(|(w, _)| *w != width) {
            *cache = Some((width, markdown::render(&self.text, width)));
        }
        f(&cache.as_ref().expect("just filled").1)
    }

    /// (line count, heading lines) at the width last drawn; zero before the
    /// first draw.
    fn measured(&self) -> (usize, Vec<usize>) {
        self.cache
            .borrow()
            .as_ref()
            .map_or((0, Vec::new()), |(_, r)| {
                (r.lines.len(), r.headings.clone())
            })
    }

    /// The furthest the page scrolls: the last line sits at the bottom.
    pub fn max_scroll(&self) -> usize {
        self.measured().0.saturating_sub(self.page.get().max(1))
    }

    pub fn scroll_by(&mut self, delta: isize) {
        let max = self.max_scroll() as isize;
        self.scroll = (self.scroll.min(max as usize) as isize + delta).clamp(0, max) as usize;
    }

    pub fn scroll_to(&mut self, line: usize) {
        self.scroll = line.min(self.max_scroll());
    }

    /// Jump to the next (`forward`) or previous heading; false when there is
    /// none that way.
    pub fn jump_heading(&mut self, forward: bool) -> bool {
        let (_, heads) = self.measured();
        let at = self.scroll.min(self.max_scroll());
        let target = if forward {
            heads.iter().copied().find(|h| *h > at)
        } else {
            heads.iter().rev().copied().find(|h| *h < at)
        };
        match target {
            // A heading too near the end to reach the top row counts as
            // "nothing further" once the page is already at the bottom.
            Some(h) if h.min(self.max_scroll()) != at => {
                self.scroll_to(h);
                true
            }
            _ => false,
        }
    }
}

fn file_mtime(p: &Path) -> Option<SystemTime> {
    std::fs::metadata(p).and_then(|m| m.modified()).ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn write(dir: &Path, name: &str, body: &str) {
        std::fs::write(dir.join(name), body).unwrap();
    }

    #[test]
    fn the_folder_comes_from_the_env_var_else_home() {
        assert_eq!(
            dir_from(Some("/tmp/j".into()), Some("/Users/m".into())),
            PathBuf::from("/tmp/j")
        );
        assert_eq!(
            dir_from(None, Some("/Users/m".into())),
            PathBuf::from("/Users/m/trading-journal")
        );
        // An empty variable is "not set".
        assert_eq!(
            dir_from(Some("".into()), Some("/Users/m".into())),
            PathBuf::from("/Users/m/trading-journal")
        );
    }

    #[test]
    fn opens_the_newest_markdown_file_and_steps_between_entries() {
        let tmp = tempfile::tempdir().unwrap();
        write(tmp.path(), "2026-11-10.md", "# Tue");
        write(tmp.path(), "2026-11-09.md", "# Mon");
        write(tmp.path(), "2026-11-11.md", "# Wed");
        write(tmp.path(), "notes.txt", "not markdown");
        write(tmp.path(), ".hidden.md", "no");
        std::fs::create_dir(tmp.path().join("old.md")).unwrap();
        let mut j = JournalState::new(tmp.path().to_path_buf());
        j.open();
        let names: Vec<_> = j.entries.iter().map(|e| e.name.as_str()).collect();
        assert_eq!(names, ["2026-11-09", "2026-11-10", "2026-11-11"]);
        assert_eq!((j.idx, j.text.as_str()), (2, "# Wed"));
        // Newer than the newest: stays put.
        assert!(!j.step_entry(1));
        assert!(j.step_entry(-1));
        assert_eq!(j.text, "# Tue");
        assert!(j.step_entry(-1));
        assert!(!j.step_entry(-1));
        assert_eq!((j.idx, j.text.as_str()), (0, "# Mon"));
    }

    #[test]
    fn a_missing_or_empty_folder_is_the_empty_state() {
        let tmp = tempfile::tempdir().unwrap();
        let mut j = JournalState::new(tmp.path().join("nope"));
        j.open();
        assert!(j.entries.is_empty() && j.error.is_none() && j.text.is_empty());
        let mut j = JournalState::new(tmp.path().to_path_buf());
        j.open();
        assert!(j.entries.is_empty() && j.error.is_none());
        assert!(!j.step_entry(1) && !j.jump_heading(true));
    }

    #[test]
    fn poll_picks_up_edits_and_new_files_without_moving() {
        let tmp = tempfile::tempdir().unwrap();
        write(tmp.path(), "2026-11-10.md", "one\n\ntwo\n\nthree\n\nfour");
        let mut j = JournalState::new(tmp.path().to_path_buf());
        j.open();
        j.page.set(2);
        j.with_rendered(40, |_| ());
        j.scroll_by(3);
        assert_eq!(j.scroll, 3);
        assert!(!j.poll(), "nothing changed");
        // The open file is edited (mtime forced forward: same-second writes
        // would otherwise look unchanged on coarse filesystems).
        let p = tmp.path().join("2026-11-10.md");
        write(
            tmp.path(),
            "2026-11-10.md",
            "one\n\ntwo\n\nthree\n\nfour\n\nfive",
        );
        let later = SystemTime::now() + std::time::Duration::from_secs(5);
        std::fs::File::options()
            .write(true)
            .open(&p)
            .unwrap()
            .set_modified(later)
            .unwrap();
        assert!(j.poll());
        assert!(j.text.ends_with("five"));
        assert_eq!(j.scroll, 3, "scroll position kept");
        // A newer day appears: counted, but the open entry doesn't change.
        write(tmp.path(), "2026-11-11.md", "# next");
        assert!(j.poll());
        assert_eq!(j.entries.len(), 2);
        assert_eq!(j.current().unwrap().name, "2026-11-10");
        // The open file is deleted: fall to a neighbour, no error.
        std::fs::remove_file(&p).unwrap();
        assert!(j.poll());
        assert_eq!(
            (j.current().unwrap().name.as_str(), j.text.as_str()),
            ("2026-11-11", "# next")
        );
    }

    #[test]
    fn scrolling_clamps_and_headings_are_jump_targets() {
        let tmp = tempfile::tempdir().unwrap();
        let body = "# A\n\none\n\n## B\n\ntwo\n\n## C\n\nthree\n\nfour\n\nfive";
        write(tmp.path(), "x.md", body);
        let mut j = JournalState::new(tmp.path().to_path_buf());
        j.open();
        j.page.set(4);
        let (n, heads) = j.with_rendered(40, |r| (r.lines.len(), r.headings.clone()));
        assert_eq!((n, heads.clone()), (16, vec![0, 5, 9]));
        assert_eq!(j.max_scroll(), 12);
        j.scroll_by(-5);
        assert_eq!(j.scroll, 0);
        assert!(j.jump_heading(true));
        assert_eq!(j.scroll, 5);
        assert!(j.jump_heading(true));
        assert_eq!(j.scroll, 9);
        assert!(!j.jump_heading(true), "no heading after C");
        assert!(j.jump_heading(false));
        assert_eq!(j.scroll, 5);
        j.scroll_to(usize::MAX);
        assert_eq!(j.scroll, 12);
        j.scroll_by(100);
        assert_eq!(j.scroll, 12);
        assert!(j.jump_heading(false));
        assert_eq!(j.scroll, 9);
    }
}
