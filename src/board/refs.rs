use anyhow::{ensure, Result};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(tag = "type", rename_all = "lowercase", deny_unknown_fields)]
pub enum BoardRef {
    Repo {
        repo: String,
        path: String,
        commit: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        start_line: Option<u32>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        end_line: Option<u32>,
    },
    Url {
        url: String,
    },
    Ledger {
        uri: String,
    },
}

impl std::fmt::Display for BoardRef {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Repo {
                repo,
                path,
                commit,
                start_line,
                end_line,
            } => {
                write!(f, "{repo}:{path}@{commit}")?;
                if let Some(start) = start_line {
                    write!(f, ":L{start}")?;
                    if let Some(end) = end_line {
                        write!(f, "-L{end}")?;
                    }
                }
                Ok(())
            }
            Self::Url { url } => f.write_str(url),
            Self::Ledger { uri } => f.write_str(uri),
        }
    }
}

pub fn validate_refs(refs: &[BoardRef]) -> Result<()> {
    ensure!(refs.len() <= 10, "refs must contain at most 10 references");
    for reference in refs {
        let bounded = |s: &str, max: usize| {
            !s.is_empty() && s.len() <= max && !s.chars().any(char::is_control)
        };
        match reference {
            BoardRef::Repo {
                repo,
                path,
                commit,
                start_line,
                end_line,
            } => {
                ensure!(
                    repo == &super::identity::validate_identifier(repo, "ref repo")?,
                    "ref repo must not have surrounding whitespace"
                );
                ensure!(
                    bounded(path, 1024) && !path.contains(['\\', ':'])
                        && path.split('/').all(|p| !matches!(p, "" | "." | "..")),
                    "ref path must be relative, at most 1024 bytes, with no empty, . or .. components, backslashes, colons, or controls"
                );
                ensure!(
                    (7..=40).contains(&commit.len())
                        && commit.bytes().all(|b| b.is_ascii_hexdigit()),
                    "ref commit must be 7-40 hexadecimal characters"
                );
                ensure!(
                    start_line.is_none_or(|n| n > 0)
                        && end_line.is_none_or(|end| start_line.is_some_and(|start| end >= start)),
                    "ref line range requires start_line >= 1 and end_line >= start_line"
                );
            }
            BoardRef::Url { url } => {
                ensure!(bounded(url, 2048) && !url.chars().any(char::is_whitespace)
                    && !url.contains('\\') && !url.starts_with("http:///") && !url.starts_with("https:///") && (url.starts_with("https://") || url.starts_with("http://")),
                    "ref URL must be http:// or https://, at most 2048 bytes, without whitespace, backslashes, or controls");
                let parsed = reqwest::Url::parse(url)?;
                ensure!(
                    parsed.host_str().is_some()
                        && parsed.username().is_empty()
                        && parsed.password().is_none(),
                    "ref URL requires a host and must not contain credentials"
                );
            }
            BoardRef::Ledger { uri } => {
                let id = uri.strip_prefix("ledger://board/").unwrap_or("");
                ensure!(
                    bounded(uri, 64)
                        && !id.starts_with('0')
                        && id.bytes().all(|b| b.is_ascii_digit())
                        && id.parse::<i64>().is_ok_and(|n| n > 0),
                    "ref ledger URI must be ledger://board/<positive message ID> in this ledger"
                );
            }
        }
    }
    Ok(())
}

pub fn notice_refs(refs: &[BoardRef]) -> String {
    refs.iter()
        .take(10)
        .map(|r| format!("  ref: {}\n", super::session::peer_preview(&r.to_string())))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::{json, Value};

    #[test]
    fn refs_validation_accept_reject_table() {
        let repo = json!({"type":"repo","repo":"daybook","path":"src/main.rs","commit":"abcdef0"});
        let mut cases: Vec<(Value, bool)> = vec![
            (repo.clone(), true),
            (
                json!({"type":"url","url":"https://example.com/a?q=1#x"}),
                true,
            ),
            (json!({"type":"url","url":"http://localhost:3000/a"}), true),
            (json!({"type":"ledger","uri":"ledger://board/123"}), true),
        ];
        for path in [
            "/etc/passwd",
            "../a",
            "a/../b",
            "a/./b",
            "a//b",
            "C:/x",
            "a\\b",
            "",
            "a\nb",
        ] {
            let mut r = repo.clone();
            r["path"] = json!(path);
            cases.push((r, false));
        }
        for bad_repo in [" daybook", "../x", "", &"r".repeat(65)] {
            let mut r = repo.clone();
            r["repo"] = json!(bad_repo);
            cases.push((r, false));
        }
        for commit in ["abcdef", "g123456", &"a".repeat(41)] {
            let mut r = repo.clone();
            r["commit"] = json!(commit);
            cases.push((r, false));
        }
        for url in [
            "javascript:alert(1)",
            "file:///etc/passwd",
            "ftp://example.com",
            "https://",
            "https:///a",
            "https://x\ny",
            "https://a b",
            "https://u:p@host",
            "//host",
            &format!("https://host/{}", "x".repeat(2048)),
        ] {
            cases.push((json!({"type":"url","url":url}), false));
        }
        for uri in [
            "ledger://session/a",
            "ledger://board/0",
            "ledger://board/01",
            "ledger://board/-1",
            "ledger://board/1?q",
            "ledger://board/9223372036854775808",
        ] {
            cases.push((json!({"type":"ledger","uri":uri}), false));
        }
        for (start, end, valid) in [
            (Some(1), None, true),
            (Some(2), Some(3), true),
            (Some(0), None, false),
            (None, Some(2), false),
            (Some(3), Some(2), false),
        ] {
            let mut r = repo.clone();
            r["start_line"] = json!(start);
            r["end_line"] = json!(end);
            cases.push((r, valid));
        }
        let mut r = repo.clone();
        r["unexpected"] = json!(true);
        cases.push((r, false));
        let mut r = repo.clone();
        r["path"] = json!("x".repeat(1025));
        cases.push((r, false));
        for (value, valid) in cases {
            let result = serde_json::from_value::<BoardRef>(value.clone())
                .map_err(anyhow::Error::from)
                .and_then(|r| validate_refs(&[r]));
            assert_eq!(result.is_ok(), valid, "{value}: {result:?}");
        }
        let r: BoardRef = serde_json::from_value(repo).unwrap();
        assert!(validate_refs(&vec![r.clone(); 10]).is_ok());
        assert!(validate_refs(&vec![r; 11]).is_err());
    }
}
