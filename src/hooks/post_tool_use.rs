use anyhow::Result;

/// Kept as a valid hook target for existing installs; the board lock touch and
/// unread-mention notices are gone.
pub async fn handle() -> Result<()> {
    Ok(())
}
