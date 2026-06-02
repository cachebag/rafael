use std::{fs::OpenOptions, io::Write, path::Path};

use anyhow::Context;
use serde::Serialize;

pub(super) async fn append_transcript<T>(
    path: &Path,
    iteration: usize,
    kind: &'static str,
    payload: &T,
) -> anyhow::Result<()>
where
    T: Serialize + ?Sized,
{
    let payload =
        serde_json::to_value(payload).context("failed to serialize transcript payload")?;
    let entry = serde_json::json!({
        "timestamp": chrono::Utc::now().to_rfc3339(),
        "iteration": iteration,
        "kind": kind,
        "payload": payload,
    });
    let mut line = serde_json::to_vec(&entry).context("failed to serialize transcript entry")?;
    line.push(b'\n');

    let path = path.to_path_buf();
    tokio::task::spawn_blocking(move || -> anyhow::Result<()> {
        let mut file = OpenOptions::new()
            .create(true)
            .append(true)
            .open(&path)
            .with_context(|| format!("failed to open {}", path.display()))?;
        file.write_all(&line)
            .with_context(|| format!("failed to append {}", path.display()))?;
        Ok(())
    })
    .await
    .context("transcript append task failed")??;

    Ok(())
}
