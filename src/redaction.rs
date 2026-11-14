//! Deterministic redaction at outbound trust boundaries. Pattern matching
//! reduces accidental disclosure; it cannot guarantee that every secret is found.

use crate::config::Config;
use regex::{Captures, Regex};
use std::sync::OnceLock;

pub const REDACTED: &str = "[REDACTED]";
const SENSITIVE_KEY_PARTS: &[&str] = &[
    "PASSWORD",
    "PASSWD",
    "SECRET",
    "TOKEN",
    "API_KEY",
    "APIKEY",
    "ACCESS_KEY",
    "PRIVATE_KEY",
];

fn patterns() -> &'static [(Regex, usize)] {
    static PATTERNS: OnceLock<Vec<(Regex, usize)>> = OnceLock::new();
    PATTERNS.get_or_init(|| {
        vec![
            (Regex::new(r#"(?s)-----BEGIN [A-Z ]*PRIVATE KEY-----.*?-----END [A-Z ]*PRIVATE KEY-----"#).unwrap(), 0),
            (Regex::new(r#"\bsk-ant-[A-Za-z0-9_-]{20,}"#).unwrap(), 0),
            (Regex::new(r#"\bsk-(?:proj-)?[A-Za-z0-9_-]{20,}"#).unwrap(), 0),
            (Regex::new(r#"\b(?:gh[pousr]_[A-Za-z0-9]{30,}|github_pat_[A-Za-z0-9_]{50,})"#).unwrap(), 0),
            (Regex::new(r#"\b(?:AKIA|ASIA)[0-9A-Z]{16}\b"#).unwrap(), 0),
            (Regex::new(r#"\bAIza[0-9A-Za-z_-]{35}"#).unwrap(), 0),
            (Regex::new(r#"\bxox[abprs]-[A-Za-z0-9-]{10,}"#).unwrap(), 0),
            (Regex::new(r#"\bBearer\s+([A-Za-z0-9._~+/-]{20,}=*)"#).unwrap(), 1),
            (Regex::new(r#"(?i)([A-Z0-9_]*(?:PASSWORD|PASSWD|SECRET|TOKEN|API_?KEY|ACCESS_KEY|PRIVATE_KEY)[A-Z0-9_]*["']?\s*[=:]\s*")((?:\\.|[^"\\\r\n])+)"#).unwrap(), 2),
            (Regex::new(r#"(?i)([A-Z0-9_]*(?:PASSWORD|PASSWD|SECRET|TOKEN|API_?KEY|ACCESS_KEY|PRIVATE_KEY)[A-Z0-9_]*\\["']\s*[=:]\s*\\")((?:\\[^"\r\n]|[^"\\\r\n])+)(\\")"#).unwrap(), 2),
            (Regex::new(r#"(?i)([A-Z0-9_]*(?:PASSWORD|PASSWD|SECRET|TOKEN|API_?KEY|ACCESS_KEY|PRIVATE_KEY)[A-Z0-9_]*\s*=\s*\\")((?:\\[^"\r\n]|[^"\\\r\n])+)(\\")"#).unwrap(), 2),
            (Regex::new(r#"(?i)([A-Z0-9_]*(?:PASSWORD|PASSWD|SECRET|TOKEN|API_?KEY|ACCESS_KEY|PRIVATE_KEY)[A-Z0-9_]*["']?\s*[=:]\s*')((?:\\.|[^'\\\r\n])+)'"#).unwrap(), 2),
            (Regex::new(r#"(?i)([A-Z0-9_]*(?:PASSWORD|PASSWD|SECRET|TOKEN|API_?KEY|ACCESS_KEY|PRIVATE_KEY)[A-Z0-9_]*\\?["']?\s*[=:]\s*)((?:\\[^"'\r\n]|[^"'\s\\])+)"#).unwrap(), 2),
            (Regex::new(r#"(?i)(\\[nrt])?([A-Z0-9._%+-]+@[A-Z0-9-]+(?:\.[A-Z0-9-]+)*\.[A-Z]{2,})\b"#).unwrap(), 2),
        ]
    })
}

fn placeholder(value: &str) -> bool {
    let lower = value.to_ascii_lowercase();
    let variable = value.strip_prefix('$').is_some_and(|name| {
        name.chars().any(|c| c.is_ascii_uppercase())
            && name
                .chars()
                .all(|c| c.is_ascii_uppercase() || c.is_ascii_digit() || c == '_')
    });
    let wrapped = [('<', '>'), ('{', '}'), ('%', '%')]
        .iter()
        .any(|(open, close)| {
            value
                .strip_prefix(*open)
                .and_then(|inner| inner.strip_suffix(*close))
                .is_some_and(|inner| {
                    !inner.is_empty()
                        && inner.chars().any(|c| c.is_ascii_uppercase())
                        && inner
                            .chars()
                            .all(|c| c.is_ascii_uppercase() || c.is_ascii_digit() || c == '_')
                })
        });
    value == REDACTED
        || variable
        || wrapped
        || matches!(
            lower.as_str(),
            "true" | "false" | "null" | "none" | "undefined"
        )
        || lower.starts_with("git@")
        || lower.starts_with("no-reply@")
        || lower.starts_with("noreply@")
        || lower.ends_with("@users.noreply.github.com")
        || [".com", ".org", ".net"]
            .iter()
            .any(|suffix| lower.ends_with(&format!("@example{suffix}")))
}

pub fn redact(input: &str) -> String {
    patterns()
        .iter()
        .fold(input.to_string(), |text, (regex, group)| {
            regex
                .replace_all(&text, |captures: &Captures| {
                    let whole = captures.get(0).unwrap();
                    let value = captures.get(*group).unwrap();
                    let key = captures.get(1).and_then(|capture| {
                        capture
                            .as_str()
                            .split(|c: char| !(c.is_ascii_alphanumeric() || c == '_'))
                            .next()
                    });
                    let numeric_metadata = *group == 2
                        && value.as_str().chars().all(|c| c.is_ascii_digit())
                        && key.is_some_and(|key| !sensitive_numeric_key(key));
                    if numeric_metadata || placeholder(value.as_str()) {
                        return whole.as_str().to_string();
                    }
                    let start = value.start() - whole.start();
                    let end = value.end() - whole.start();
                    format!(
                        "{}{}{}",
                        &whole.as_str()[..start],
                        REDACTED,
                        &whole.as_str()[end..]
                    )
                })
                .into_owned()
        })
}

/// Redact a JSON string using its field name as credential context.
pub(crate) fn sensitive_key(key: &str) -> bool {
    let key = key.to_ascii_uppercase();
    SENSITIVE_KEY_PARTS.iter().any(|part| {
        key == *part
            || key.ends_with(part)
            || key.starts_with(&format!("{part}_"))
            || key.contains(&format!("_{part}_"))
    })
}

// Numeric metadata such as password_attempts and api_key_id stays numeric.
// Only keys whose full name ends in a credential name carry numeric secrets.
pub(crate) fn sensitive_numeric_key(key: &str) -> bool {
    let key = key.to_ascii_uppercase();
    SENSITIVE_KEY_PARTS.iter().any(|part| key.ends_with(part))
}

/// Redact a JSON string using its field name as credential context.
pub fn redact_named_value(key: Option<&str>, value: &str, config: &Config) -> String {
    if key.is_some_and(sensitive_key) && !placeholder(value) {
        REDACTED.to_string()
    } else {
        redact_for_config(value, config)
    }
}

pub fn redact_for_config(input: &str, config: &Config) -> String {
    let mut output = input.to_string();
    for secret in [
        config.push.token.as_deref(),
        config.server.admin_token.as_deref(),
        config.insights.api_key.as_deref(),
    ]
    .into_iter()
    .flatten()
    {
        let secret = secret.trim();
        if secret.len() >= 6 {
            output = output.replace(secret, REDACTED);
        }
    }
    for key in [
        "DAYBOOK_INSIGHTS_API_KEY",
        "OPENAI_API_KEY",
        "ANTHROPIC_API_KEY",
    ] {
        if let Ok(secret) = std::env::var(key) {
            let secret = secret.trim();
            if secret.len() >= 6 {
                output = output.replace(secret, REDACTED);
            }
        }
    }
    redact(&output)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn redacts_known_credential_formats() {
        let aws_access_key = ["AKIA", "1234567890ABCDEF"].concat();
        let secrets = [
            "sk-ant-abcdefghijklmnopqrstuvwx",
            "sk-proj-abcdefghijklmnopqrstuvwx",
            "ghp_abcdefghijklmnopqrstuvwxyz123456",
            "github_pat_abcdefghijklmnopqrstuvwxyzABCDEFGHIJKLMNOPQRSTUVWX",
            aws_access_key.as_str(),
            "AIzaabcdefghijklmnopqrstuvwxyzABCDEFGHI",
            "xoxb-abcdefghijkl",
        ];
        for secret in secrets {
            let output = redact(&format!("before {secret} after"));
            assert!(!output.contains(secret), "leaked {secret} in {output}");
            assert!(output.contains(REDACTED), "unmatched {secret}");
        }

        let private_key = "-----BEGIN PRIVATE KEY-----\nabc123\n-----END PRIVATE KEY-----";
        assert!(!redact(private_key).contains("abc123"));
        assert!(!redact("Authorization: Bearer abcdefghijklmnopqrstuvwxyz")
            .contains("abcdefghijklmnopqrstuvwxyz"));
        assert!(!redact("DATABASE_PASSWORD=correct-horse-secret").contains("correct-horse-secret"));
        assert!(!redact("contact person@company.internal").contains("person@company.internal"));
    }

    #[test]
    fn redacts_credential_assignments_without_leaking_suffixes() {
        let cases = [
            (
                r#"{"password":"correct-horse-secret"}"#,
                "correct-horse-secret",
            ),
            (
                r#"{"api_key":"vendor-credential-12345"}"#,
                "vendor-credential-12345",
            ),
            ("DATABASE_PASSWORD=12345678", "12345678"),
            ("DATABASE_PASSWORD='$trongPassword9'", "$trongPassword9"),
            (
                r#"DATABASE_PASSWORD="my secret password""#,
                "my secret password",
            ),
            (
                r#"DATABASE_PASSWORD="correct horse secret""#,
                "horse secret",
            ),
            (
                r#"DATABASE_PASSWORD="my secret\password""#,
                r#"my secret\password"#,
            ),
            (
                r#"{\"password\":\"my secret\\password\"}"#,
                r#"my secret\\password"#,
            ),
            ("PASSWORD=abc12", "abc12"),
            (r#"password: "{s3cr3t}""#, "{s3cr3t}"),
            ("PASSWORD=abc\\def123", "abc\\def123"),
            ("TOKEN=part1,part2", "part1,part2"),
            ("API_KEY=ab{cd}", "ab{cd}"),
            ("PASSWORD=$12345678", "$12345678"),
            ("PASSWORD=<12345678>", "<12345678>"),
        ];
        for (input, secret) in cases {
            let output = redact(input);
            assert!(!output.contains(secret), "leaked {secret} in {output}");
            assert!(output.contains(REDACTED), "unmatched {input}");
        }
        assert_eq!(
            redact(r#"DATABASE_PASSWORD="my secret\password""#),
            r#"DATABASE_PASSWORD="[REDACTED]""#
        );
        assert_eq!(
            redact(r#"{\"password\":\"my secret\\password\"}"#),
            r#"{\"password\":\"[REDACTED]\"}"#
        );
        assert_eq!(
            redact(r#"{"cmd":"export TOKEN=abc"}"#),
            r#"{"cmd":"export TOKEN=[REDACTED]"}"#
        );
        assert_eq!(
            redact(r#"{"text":"export PASSWORD=\"correct horse secret\""}"#),
            r#"{"text":"export PASSWORD=\"[REDACTED]\""}"#
        );
    }
    #[test]
    fn preserves_numeric_token_counts() {
        assert_eq!(
            redact("tokens: 1234 cache_tokens: 500 token_count: 99"),
            "tokens: 1234 cache_tokens: 500 token_count: 99"
        );
        assert_eq!(redact("PASSWORD=12345678"), "PASSWORD=[REDACTED]");
        assert_eq!(
            redact("Authorization: Bearer 12345678901234567890"),
            "Authorization: Bearer [REDACTED]"
        );
    }

    #[test]
    fn preserves_placeholders_and_example_addresses() {
        assert_eq!(
            redact("DATABASE_PASSWORD=$DATABASE_PASSWORD"),
            "DATABASE_PASSWORD=$DATABASE_PASSWORD"
        );
        assert_eq!(
            redact("contact person@example.org"),
            "contact person@example.org"
        );
    }

    #[test]
    fn redacts_configured_secrets_before_patterns() {
        let mut config = Config::default();
        config.push.token = Some("prefix-ghp_abcdefghijklmnopqrstuvwxyz123456".into());
        let output =
            redact_for_config("token prefix-ghp_abcdefghijklmnopqrstuvwxyz123456", &config);
        assert_eq!(output, "token [REDACTED]");
    }
}
