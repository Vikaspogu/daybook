use anyhow::{bail, Result};
use std::path::{Component, Path, PathBuf};

pub const MAX_GLOB_SEGMENTS: usize = 16;
pub const MAX_GLOB_LEN: usize = 512;

/// Canonicalize a path component-by-component so symlinks before `..` are followed on disk
/// even when leaf components do not exist yet.
fn canonicalize_partial(path: &Path) -> PathBuf {
    let mut acc = PathBuf::new();
    for comp in path.components() {
        match comp {
            Component::CurDir => {}
            Component::ParentDir => {
                let next = acc.join("..");
                if let Ok(c) = std::fs::canonicalize(&next) {
                    acc = c;
                } else {
                    acc.pop();
                }
            }
            other => {
                acc.push(other.as_os_str());
                if acc.exists() {
                    if let Ok(c) = std::fs::canonicalize(&acc) {
                        acc = c;
                    }
                }
            }
        }
    }
    acc
}

/// Normalize a repo-relative path or glob pattern and enforce segment/length bounds.
/// When `worktree_root` is provided and `raw` has no wildcards, resolves symlinks
/// and rejects paths escaping `worktree_root`.
pub fn normalize_repo_path(raw: &str, worktree_root: Option<&Path>) -> Result<String> {
    validate_and_normalize(raw, worktree_root, false)
}

/// Resolve a file path from an editor/tool hook (`cwd` + `raw`).
/// Returns `Ok(None)` when the resolved path lies outside `worktree_root` (not a repo file),
/// `Ok(Some(rel))` when inside `worktree_root` and valid, and `Err(...)` when inside `worktree_root`
/// but invalid (e.g. NUL bytes, wildcards, or exceeding length/segment bounds).
pub fn resolve_hook_file_path(
    cwd: &Path,
    worktree_root: &Path,
    raw: &str,
) -> Result<Option<String>> {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return Ok(None);
    }
    if trimmed.contains('\0') {
        bail!("path must not contain NUL bytes");
    }
    let canon_root =
        std::fs::canonicalize(worktree_root).unwrap_or_else(|_| worktree_root.to_path_buf());
    let p = Path::new(trimmed);
    let anchored = if p.is_absolute() {
        p.to_path_buf()
    } else {
        cwd.join(p)
    };
    let resolved = canonicalize_partial(&anchored);
    if !resolved.starts_with(&canon_root) {
        return Ok(None);
    }
    let rel = resolved.strip_prefix(&canon_root).unwrap_or(&resolved);
    let rel_str = rel.to_string_lossy().replace('\\', "/");
    if rel_str.is_empty() {
        return Ok(None);
    }
    normalize_repo_path(&rel_str, None).map(Some)
}

/// Validate and normalize a glob pattern (may contain `*`, `**`, `?`).
pub fn validate_glob_pattern(raw: &str, worktree_root: Option<&Path>) -> Result<String> {
    validate_and_normalize(raw, worktree_root, true)
}

fn validate_and_normalize(
    raw: &str,
    worktree_root: Option<&Path>,
    allow_wildcards: bool,
) -> Result<String> {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        bail!("path must not be empty");
    }
    if trimmed.contains('\0') {
        bail!("path must not contain NUL bytes");
    }

    let slash_norm = trimmed.replace('\\', "/");
    let has_wildcards = slash_norm.contains(['*', '?']);
    if has_wildcards && !allow_wildcards {
        bail!("file path must not contain glob wildcards: {trimmed}");
    }

    let rel_str = if let Some(root) = worktree_root {
        let canon_root = std::fs::canonicalize(root).unwrap_or_else(|_| root.to_path_buf());
        let root_slash = canon_root.to_string_lossy().replace('\\', "/");
        let raw_root_slash = root.to_string_lossy().replace('\\', "/");

        if !has_wildcards {
            let p = Path::new(&slash_norm);
            let candidate = if p.is_absolute() {
                p.to_path_buf()
            } else {
                canon_root.join(p)
            };
            // Reject explicit `..` components before filesystem touch
            if p.components().any(|c| matches!(c, Component::ParentDir)) {
                bail!("path traversal ('..') is not allowed: {trimmed}");
            }
            let canon_target = canonicalize_partial(&candidate);
            if !canon_target.starts_with(&canon_root) {
                bail!(
                    "path '{}' resolves outside worktree root '{}'",
                    trimmed,
                    canon_root.display()
                );
            }
            let stripped = canon_target
                .strip_prefix(&canon_root)
                .unwrap_or(&canon_target);
            stripped.to_string_lossy().replace('\\', "/")
        } else if slash_norm.starts_with('/') {
            let s = slash_norm
                .strip_prefix(&format!("{root_slash}/"))
                .or_else(|| slash_norm.strip_prefix(&format!("{raw_root_slash}/")))
                .ok_or_else(|| {
                    anyhow::anyhow!(
                        "absolute glob '{}' is outside worktree root '{}'",
                        trimmed,
                        root.display()
                    )
                })?;
            s.to_string()
        } else {
            slash_norm
        }
    } else {
        if slash_norm.starts_with('/') {
            bail!("path must be repo-relative, not absolute: {trimmed}");
        }
        slash_norm
    };

    let mut clean = rel_str.as_str();
    while let Some(rest) = clean.strip_prefix("./") {
        clean = rest;
    }
    clean = clean.trim_matches('/');

    if clean.is_empty() {
        bail!("path resolves to repository root; specify a file or subtree glob");
    }
    if clean.len() > MAX_GLOB_LEN {
        bail!("path exceeds maximum length of {MAX_GLOB_LEN} characters");
    }

    let mut segments: Vec<&str> = Vec::new();
    for seg in clean.split('/') {
        if seg.is_empty() {
            continue;
        }
        if seg == "." {
            bail!("invalid path segment in '{trimmed}'");
        }
        if seg == ".." {
            bail!("path traversal ('..') is not allowed: {trimmed}");
        }
        // Collapse consecutive `**` segments to keep DP state tiny
        if seg == "**" && segments.last() == Some(&"**") {
            continue;
        }
        segments.push(seg);
    }

    if segments.len() > MAX_GLOB_SEGMENTS {
        bail!(
            "path has {} segments, exceeding maximum of {}",
            segments.len(),
            MAX_GLOB_SEGMENTS
        );
    }

    Ok(segments.join("/"))
}

/// Check whether two single-segment wildcard patterns (`*`, `?`) can match the same segment string.
pub fn segment_patterns_overlap(a: &str, b: &str) -> bool {
    let ac: Vec<char> = a.chars().collect();
    let bc: Vec<char> = b.chars().collect();
    let m = ac.len();
    let n = bc.len();
    let mut dp = vec![vec![false; n + 1]; m + 1];
    dp[0][0] = true;

    for i in 0..=m {
        for j in 0..=n {
            if !dp[i][j] {
                continue;
            }
            if i < m && ac[i] == '*' {
                dp[i + 1][j] = true;
                if j < n {
                    dp[i][j + 1] = true;
                }
            }
            if j < n && bc[j] == '*' {
                dp[i][j + 1] = true;
                if i < m {
                    dp[i + 1][j] = true;
                }
            }
            if i < m
                && j < n
                && (ac[i] == '*' || bc[j] == '*' || ac[i] == '?' || bc[j] == '?' || ac[i] == bc[j])
            {
                dp[i + 1][j + 1] = true;
            }
        }
    }

    dp[m][n]
}

/// Exact 2D segment-level DP intersection for normalized globs `x` and `y`.
fn raw_globs_overlap(x: &str, y: &str) -> bool {
    let sx: Vec<&str> = x.split('/').collect();
    let sy: Vec<&str> = y.split('/').collect();
    let m = sx.len();
    let n = sy.len();
    let mut dp = vec![vec![false; n + 1]; m + 1];
    dp[0][0] = true;

    for i in 0..=m {
        for j in 0..=n {
            if !dp[i][j] {
                continue;
            }
            if i < m && sx[i] == "**" {
                dp[i + 1][j] = true;
                if j < n {
                    dp[i][j + 1] = true;
                }
            }
            if j < n && sy[j] == "**" {
                dp[i][j + 1] = true;
                if i < m {
                    dp[i + 1][j] = true;
                }
            }
            if i < m
                && j < n
                && (sx[i] == "**" || sy[j] == "**" || segment_patterns_overlap(sx[i], sy[j]))
            {
                dp[i + 1][j + 1] = true;
            }
        }
    }

    dp[m][n]
}

/// Returns `true` if `glob` matches concrete path `path` (or `glob` is a directory prefix of `path`).
pub fn glob_matches_path(glob: &str, path: &str) -> bool {
    let Ok(ng) = validate_glob_pattern(glob, None) else {
        return false;
    };
    let Ok(np) = validate_glob_pattern(path, None) else {
        return false;
    };
    raw_globs_overlap(&ng, &np)
        || (!ng.contains(['*', '?']) && raw_globs_overlap(&format!("{ng}/**"), &np))
}

/// Symmetric, exact overlap check between two glob patterns (including directory prefixes).
pub fn globs_overlap(a: &str, b: &str) -> bool {
    let Ok(na) = validate_glob_pattern(a, None) else {
        return false;
    };
    let Ok(nb) = validate_glob_pattern(b, None) else {
        return false;
    };
    let a_plain = !na.contains(['*', '?']);
    let b_plain = !nb.contains(['*', '?']);

    raw_globs_overlap(&na, &nb)
        || (a_plain && raw_globs_overlap(&format!("{na}/**"), &nb))
        || (b_plain && raw_globs_overlap(&na, &format!("{nb}/**")))
        || (a_plain && b_plain && raw_globs_overlap(&format!("{na}/**"), &format!("{nb}/**")))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn exact_and_directory_prefix_overlap() {
        assert!(globs_overlap("src/ledger/mod.rs", "src/ledger/mod.rs"));
        assert!(globs_overlap("src/ledger", "src/ledger/mod.rs"));
        assert!(globs_overlap("src/ledger/mod.rs", "src/ledger"));
        assert!(!globs_overlap("src/ledger/mod.rs", "src/server/router.rs"));
    }

    #[test]
    fn wildcard_and_double_star_overlap_is_symmetric() {
        assert!(globs_overlap("src/**/*.rs", "src/ledger/*.rs"));
        assert!(globs_overlap("src/ledger/*.rs", "src/**/*.rs"));
        assert!(globs_overlap("src/**/mod.rs", "src/ledger"));
        assert!(globs_overlap("src/ledger", "src/**/mod.rs"));
        assert!(globs_overlap("*.rs", "main.rs"));
        assert!(!globs_overlap("src/**/*.ts", "src/ledger/*.rs"));
        assert!(!globs_overlap("web/src/**", "src/server/**"));
    }

    #[test]
    fn glob_matches_path_handles_subtrees_and_wildcards() {
        assert!(glob_matches_path("src/server/**", "src/server/router.rs"));
        assert!(glob_matches_path("src/server", "src/server/router.rs"));
        assert!(glob_matches_path("src/*/router.rs", "src/server/router.rs"));
        assert!(!glob_matches_path("src/server/**", "web/src/App.tsx"));
    }

    #[test]
    fn normalize_rejects_traversal_and_oversized_globs() {
        assert!(normalize_repo_path("../secret.txt", None).is_err());
        assert!(normalize_repo_path("src/../../etc/passwd", None).is_err());
        assert!(validate_glob_pattern("/etc/passwd", None).is_err());
        assert!(validate_glob_pattern("a/b/c/d/e/f/g/h/i/j/k/l/m/n/o/p/q", None).is_err());
        assert_eq!(
            validate_glob_pattern("./src///**/**/mod.rs/", None).unwrap(),
            "src/**/mod.rs"
        );
    }

    #[test]
    fn normalize_strips_worktree_root_and_blocks_outside_paths() {
        let tmp = tempfile::TempDir::new().unwrap();
        let wt = tmp.path().join("repo");
        std::fs::create_dir_all(wt.join("src")).unwrap();
        let file = wt.join("src/main.rs");
        std::fs::write(&file, "fn main() {}").unwrap();

        let rel = normalize_repo_path(&file.to_string_lossy(), Some(&wt)).unwrap();
        assert_eq!(rel, "src/main.rs");

        let outside = tmp.path().join("outside.txt");
        std::fs::write(&outside, "nope").unwrap();
        assert!(normalize_repo_path(&outside.to_string_lossy(), Some(&wt)).is_err());
    }
}
