use anyhow::Result;

/// `daybook token`: mint (default), --list, or --revoke <label>.
pub fn run(list: bool, revoke: Option<String>, label: Option<String>) -> Result<()> {
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
    if replacing_active {
        eprintln!("[daybook] Replacing active token '{label}': the old secret stops working");
    }
    let secret = crate::ledger::generate_secret()?;
    ledger.mint_token(&label, &crate::ledger::sha256_hex(secret.as_bytes()))?;
    println!("{secret}");
    eprintln!("[daybook] Token '{label}' minted. Store it now — the secret is never shown again.");
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
}
