use anyhow::{Context, Result};

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

    let label = label.unwrap_or_else(|| crate::push::machine_label(&config));
    let replacing_active = ledger
        .list_tokens()?
        .into_iter()
        .any(|(l, _, _, revoked)| l == label && !revoked);
    if replacing_active {
        eprintln!("[daybook] Replacing active token '{label}': the old secret stops working");
    }
    let secret = format!("dbk_{}", random_hex(32)?);
    ledger.mint_token(&label, &crate::ledger::sha256_hex(secret.as_bytes()))?;
    println!("{secret}");
    eprintln!("[daybook] Token '{label}' minted. Store it now — the secret is never shown again.");
    Ok(())
}

/// n random bytes from the OS, hex-encoded.
fn random_hex(n: usize) -> Result<String> {
    use std::io::Read;
    let mut buf = vec![0u8; n];
    std::fs::File::open("/dev/urandom")
        .and_then(|mut f| f.read_exact(&mut buf))
        .context("read /dev/urandom")?;
    Ok(buf.iter().map(|b| format!("{b:02x}")).collect())
}

#[cfg(test)]
mod tests {
    #[test]
    fn test_random_hex_is_hex_and_right_length() {
        let hex = super::random_hex(8).unwrap();
        assert_eq!(hex.len(), 16);
        assert!(hex.chars().all(|c| c.is_ascii_hexdigit()));
        // Two draws differ (cosmic-ray odds ignored).
        assert_ne!(hex, super::random_hex(8).unwrap());
    }
}
