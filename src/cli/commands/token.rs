use anyhow::Result;

/// `daybook token`: mint (default), --list, --revoke <label>, or --value <TOKEN> (GitOps: ensure a
/// known token from a secret store exists in the ledger instead of minting a random one).
fn validate_token_value(v: &str) -> Result<()> {
    match v.strip_prefix("dbk_") {
        Some(body) if body.len() == 64 && body.bytes().all(|c| c.is_ascii_hexdigit()) => Ok(()),
        _ => anyhow::bail!("--value must be a dbk_ token (68 hex chars; mint one with `daybook token`)"),
    }
}

pub fn run(list: bool, revoke: Option<String>, label: Option<String>, value: Option<String>) -> Result<()> {
    let config = crate::config::load_config()?;
    let ledger = crate::ledger::Ledger::open(&config.server.db_path)?;

    if let Some(label) = revoke {
        if ledger.revoke_token(&label)? {
            eprintln!("[daybook] Revoked token '{label}'");
            return Ok(());
        }
        anyhow::bail!("no active token labeled '{label}'");
    }

    if list {
        println!(
            "{:<24} {:<24} {:<24} revoked",
            "label", "created", "last_used"
        );
        for (label, created, last_used, revoked) in ledger.list_tokens()? {
            println!(
                "{:<24} {:<24} {:<24} {}",
                label, created, last_used, revoked
            );
        }
        return Ok(());
    }

    let label = label
        .unwrap_or_else(|| crate::push::machine_label(&config))
        .trim()
        .to_string();
    if label.is_empty() || label.len() > 64 || label.contains(':') {
        anyhow::bail!("label must be 1-64 chars and must not contain ':'");
    }
    let replacing_active = ledger
        .list_tokens()?
        .into_iter()
        .any(|(l, _, _, revoked)| l == label && !revoked);
    if replacing_active && value.is_none() {
        eprintln!("[daybook] Replacing active token '{label}': the old secret stops working");
    }
    let secret = match value.as_deref() {
        Some(v) => {
            validate_token_value(v)?;
            v.to_string()
        }
        None => crate::ledger::generate_secret()?,
    };
    ledger.mint_token(&label, &crate::ledger::sha256_hex(secret.as_bytes()))?;
    if value.is_some() {
        // GitOps mode: the value came from a secret store; nothing to store.
        eprintln!("[daybook] Token '{label}' ensured in the ledger.");
    } else {
        println!("{secret}");
        eprintln!("[daybook] Token '{label}' minted. Store it now — the secret is never shown again.");
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    #[test]
    fn test_generate_secret_shape() {
        let s = crate::ledger::generate_secret().unwrap();
        assert!(s.starts_with("dbk_") && s.len() == 68);
        assert_ne!(s, crate::ledger::generate_secret().unwrap());
    }

    #[test]
    fn test_validate_token_value() {
        let ok = format!("dbk_{}", "a".repeat(64));
        assert!(super::validate_token_value(&ok).is_ok());
        assert!(super::validate_token_value(&format!("dbk_{}", "z".repeat(64))).is_err()); // non-hex
        assert!(super::validate_token_value(&format!("dbk_{}", "a".repeat(63))).is_err()); // short
        assert!(super::validate_token_value(&"d".repeat(68)).is_err()); // 68 hex chars, no dbk_ prefix
    }
}
