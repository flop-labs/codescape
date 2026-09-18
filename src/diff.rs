//! Git diff as an overlay: which lines of which files a change touches.
//!
//! One unified-diff parser serves both sources. `--diff <rev>` compares a
//! revision against the working tree, which is exactly what the map renders,
//! so line numbers always line up. `--diff <a>..<b>` and `--pr <n>` compare
//! two revisions instead; if the working tree is not `<b>`, marks are clamped
//! to the file on disk and the count of clamped lines is reported.

use crate::overlay::{FileOverlay, FileState, Overlay, M_ADDED, M_CHANGED, M_GONE, M_NONE};
use crate::scan::SourceFile;
use std::collections::HashMap;
use std::path::Path;
use std::process::Command;

/// Runs a command and returns stdout, or `None` if it could not be run or failed.
fn capture(cmd: &mut Command) -> Option<String> {
    let out = cmd.output().ok()?;
    if !out.status.success() && out.stdout.is_empty() {
        return None;
    }
    Some(String::from_utf8_lossy(&out.stdout).into_owned())
}

/// Reads the patch for `spec`, which is a revision, a `a..b` range, or empty
/// for the working tree against HEAD.
pub fn patch_for(root: &Path, spec: &str) -> Option<String> {
    let mut cmd = Command::new("git");
    cmd.arg("-C").arg(root).args([
        "--no-pager",
        "-c",
        "core.quotePath=false",
        "diff",
        "--no-color",
        "--no-ext-diff",
        "-M",
        "-U0",
    ]);
    if !spec.is_empty() {
        cmd.arg(spec);
    }
    capture(&mut cmd)
}

/// `owner/repo` from a remote URL. The origin here is a proxy scheme rather
/// than a github.com URL, so `gh` cannot infer the repository on its own.
fn slug(remote: &str) -> Option<String> {
    let remote = remote.trim().trim_end_matches('/').trim_end_matches(".git");
    let mut parts = remote.rsplit(['/', ':']);
    let repo = parts.next()?;
    let owner = parts.next()?;
    if owner.is_empty() || repo.is_empty() {
        return None;
    }
    Some(format!("{owner}/{repo}"))
}

/// Reads a pull request patch through `gh`, which does not need the branch locally.
pub fn patch_for_pr(root: &Path, pr: u32) -> Option<String> {
    let pr = pr.to_string();
    let remote = capture(
        Command::new("git")
            .arg("-C")
            .arg(root)
            .args(["remote", "get-url", "origin"]),
    )
    .unwrap_or_default();
    if let Some(slug) = slug(&remote) {
        if let Some(patch) = capture(
            Command::new("gh")
                .current_dir(root)
                .args(["pr", "diff", &pr, "-R", &slug]),
        ) {
            return Some(patch);
        }
    }
    capture(
        Command::new("gh")
            .current_dir(root)
            .args(["pr", "diff", &pr]),
    )
}

/// Strips the `a/` or `b/` prefix and any quoting from a diff header path.
fn header_path(s: &str) -> Option<String> {
    let s = s.trim();
    if s == "/dev/null" {
        return None;
    }
    let s = s
        .strip_prefix('"')
        .and_then(|s| s.strip_suffix('"'))
        .unwrap_or(s);
    let s = s
        .strip_prefix("a/")
        .or_else(|| s.strip_prefix("b/"))
        .unwrap_or(s);
    Some(s.to_string())
}

/// `@@ -a,b +c,d @@` — returns the head-side start line and count.
fn hunk_head(line: &str) -> Option<(usize, usize)> {
    let rest = line.strip_prefix("@@ ")?;
    let plus = rest.split('+').nth(1)?;
    let span = plus.split(' ').next()?;
    let mut parts = span.split(',');
    let start: usize = parts.next()?.parse().ok()?;
    let count: usize = match parts.next() {
        Some(c) => c.parse().ok()?,
        None => 1,
    };
    Some((start, count))
}

#[derive(Default)]
struct Pending {
    state: Option<FileState>,
    added: usize,
    removed: usize,
    /// (head line, mark), 1-based; resolved against the scanned file later.
    marks: Vec<(usize, u8)>,
}

/// Parses a unified diff into per-path marks.
fn parse(patch: &str) -> HashMap<String, Pending> {
    let mut out: HashMap<String, Pending> = HashMap::new();
    let mut path: Option<String> = None;
    let mut state = FileState::Modified;
    // Head-side line number of the next body line, and the current hunk's buffer.
    let mut head_line = 0usize;
    let mut hunk_added: Vec<usize> = Vec::new();
    let mut hunk_removed_at: Vec<usize> = Vec::new();
    // `--- x` and `+++ x` are headers only before the first hunk; inside one
    // they are ordinary removed or added lines whose text happens to start so.
    let mut in_hunk = false;

    // Closes the open hunk: a hunk with both sides is a change, not an addition.
    fn flush(
        out: &mut HashMap<String, Pending>,
        path: &Option<String>,
        added: &mut Vec<usize>,
        removed_at: &mut Vec<usize>,
    ) {
        let Some(p) = path else {
            added.clear();
            removed_at.clear();
            return;
        };
        let entry = out.entry(p.clone()).or_default();
        let mark = if removed_at.is_empty() {
            M_ADDED
        } else {
            M_CHANGED
        };
        for l in added.drain(..) {
            entry.marks.push((l, mark));
        }
        if mark == M_ADDED {
            removed_at.clear();
        }
        // A removal with nothing added in its place leaves no line to colour,
        // so the mark goes on the line that closed the gap.
        for l in removed_at.drain(..) {
            entry.marks.push((l.max(1), M_GONE));
        }
    }

    for line in patch.lines() {
        if let Some(rest) = line.strip_prefix("diff --git ") {
            flush(&mut out, &path, &mut hunk_added, &mut hunk_removed_at);
            // `a/x b/y`; take the head side, which is everything after " b/".
            path = rest.rsplit_once(" b/").and_then(|(_, b)| header_path(b));
            state = FileState::Modified;
            in_hunk = false;
            if let Some(p) = &path {
                out.entry(p.clone()).or_default();
            }
            continue;
        }
        if line.starts_with("new file mode") {
            state = FileState::Added;
        } else if line.starts_with("deleted file mode") {
            state = FileState::Deleted;
        } else if !in_hunk && line.starts_with("--- ") {
            // The base-side header carries no head-side information.
        } else if !in_hunk && line.starts_with("+++ ") {
            if let Some(p) = header_path(&line[4..]) {
                path = Some(p);
            }
        } else if line.starts_with("@@ ") {
            flush(&mut out, &path, &mut hunk_added, &mut hunk_removed_at);
            if let Some((start, _)) = hunk_head(line) {
                head_line = start;
            }
            in_hunk = true;
        } else if let Some(p) = &path {
            let entry = out.entry(p.clone()).or_default();
            entry.state = Some(state);
            match line.as_bytes().first() {
                Some(b'+') => {
                    entry.added += 1;
                    hunk_added.push(head_line);
                    head_line += 1;
                }
                Some(b'-') => {
                    entry.removed += 1;
                    hunk_removed_at.push(head_line);
                }
                Some(b' ') => head_line += 1,
                _ => {}
            }
        }
    }
    flush(&mut out, &path, &mut hunk_added, &mut hunk_removed_at);
    out
}

/// Builds an overlay for `sources` from a patch. Returns the overlay and the
/// number of marks dropped because the local file has no such line.
pub fn overlay(patch: &str, sources: &[SourceFile], label: String) -> (Overlay, usize) {
    let parsed = parse(patch);
    let index: HashMap<&str, usize> = sources
        .iter()
        .enumerate()
        .map(|(i, s)| (s.path.as_str(), i))
        .collect();

    let mut ov = Overlay::empty(sources.len());
    ov.label = label;
    let mut dropped = 0usize;
    for (path, p) in parsed {
        let state = p.state.unwrap_or(FileState::Modified);
        let Some(&i) = index.get(path.as_str()) else {
            // Deleted, or a path the scan filters out (lockfiles, binaries).
            if state == FileState::Deleted || p.removed > 0 {
                ov.deleted.push((path, p.removed));
            }
            continue;
        };
        if state == FileState::Deleted {
            ov.deleted.push((path, p.removed));
            continue;
        }
        let lines = sources[i].line_count();
        let mut marks = vec![M_NONE; lines];
        for (l, m) in p.marks {
            match marks.get_mut(l - 1) {
                // A line added and then noted as a removal point stays added.
                Some(slot) if *slot == M_NONE || m != M_GONE => *slot = m,
                Some(_) => {}
                None => dropped += 1,
            }
        }
        ov.files[i] = Some(FileOverlay {
            state,
            added: p.added,
            removed: p.removed,
            marks,
        });
    }
    ov.deleted.sort_by(|a, b| b.1.cmp(&a.1));
    (ov, dropped)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::overlay::{M_ADDED, M_CHANGED, M_GONE, M_NONE};

    /// A file of `n` blank-ish lines, so marks can be addressed by line number.
    fn source(path: &str, n: usize) -> SourceFile {
        let text = vec![b'x'; n];
        SourceFile {
            path: path.to_string(),
            class: vec![1u8; n],
            text,
            line_starts: (0..n as u32).collect(),
        }
    }

    const PATCH: &str = "\
diff --git a/src/a.rs b/src/a.rs
index 1111111..2222222 100644
--- a/src/a.rs
+++ b/src/a.rs
@@ -2,0 +3,2 @@ fn x() {
+    let a = 1;
+    let b = 2;
@@ -10,2 +12,1 @@ fn y() {
-    old();
-    older();
+    new();
@@ -20,1 +21,0 @@ fn z() {
-    gone();
diff --git a/src/new.rs b/src/new.rs
new file mode 100644
index 0000000..3333333
--- /dev/null
+++ b/src/new.rs
@@ -0,0 +1,2 @@
+fn fresh() {}
+// added
diff --git a/src/old.rs b/src/old.rs
deleted file mode 100644
index 4444444..0000000
--- a/src/old.rs
+++ /dev/null
@@ -1,3 +0,0 @@
-fn a() {}
-fn b() {}
-fn c() {}
";

    #[test]
    fn remote_urls_reduce_to_a_gh_slug() {
        assert_eq!(
            slug("entire://aws-ap-southeast-2.entire.io/gh/flop-labs/flop-core").as_deref(),
            Some("flop-labs/flop-core")
        );
        assert_eq!(
            slug("git@github.com:flop-labs/flop-core.git").as_deref(),
            Some("flop-labs/flop-core")
        );
        assert_eq!(
            slug("https://github.com/flop-labs/flop-core/").as_deref(),
            Some("flop-labs/flop-core")
        );
    }

    #[test]
    fn marks_additions_changes_and_removals() {
        let sources = vec![source("src/a.rs", 40), source("src/new.rs", 2)];
        let (ov, dropped) = overlay(PATCH, &sources, "test".into());
        assert_eq!(dropped, 0);

        let a = ov.files[0].as_ref().expect("src/a.rs is marked");
        assert_eq!(a.state, FileState::Modified);
        assert_eq!((a.added, a.removed), (3, 3));
        // Hunk with no removals: plain additions on head lines 3 and 4.
        assert_eq!(a.marks[2], M_ADDED);
        assert_eq!(a.marks[3], M_ADDED);
        // Hunk replacing two lines with one: the surviving line is a change.
        assert_eq!(a.marks[11], M_CHANGED);
        // Pure deletion: the mark sits on the line that closed the gap.
        assert_eq!(a.marks[20], M_GONE);
        assert_eq!(a.marks[0], M_NONE);
    }

    #[test]
    fn new_files_are_added_and_deleted_files_have_no_tile() {
        let sources = vec![source("src/a.rs", 40), source("src/new.rs", 2)];
        let (ov, _) = overlay(PATCH, &sources, "test".into());

        let n = ov.files[1].as_ref().expect("src/new.rs is marked");
        assert_eq!(n.state, FileState::Added);
        assert_eq!(n.marks, vec![M_ADDED, M_ADDED]);

        // src/old.rs is gone from the working tree, so it gets no tile.
        assert_eq!(ov.deleted.len(), 1);
        assert_eq!(ov.deleted[0], ("src/old.rs".to_string(), 3));
    }

    #[test]
    fn marks_past_the_end_of_the_scanned_file_are_counted_not_lost_silently() {
        // Head revision is longer than the file on disk: a stale working tree.
        let sources = vec![source("src/a.rs", 5), source("src/new.rs", 2)];
        let (ov, dropped) = overlay(PATCH, &sources, "test".into());
        assert_eq!(dropped, 4);
        // The file still reads as changed, with its real counts.
        let a = ov.files[0].as_ref().expect("still marked");
        assert_eq!((a.added, a.removed), (3, 3));
    }
}
