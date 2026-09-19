//! Optional compositor placement for a popup on Niri's otherwise tiled desktop.

use anyhow::{Context as _, Result};
use gpui_kit::{Pixels, Size};
use std::time::Duration;
use tokio::{process::Command, runtime::Handle, task::JoinHandle};

const COMMAND_TIMEOUT: Duration = Duration::from_secs(2);
const MAP_ATTEMPTS: usize = 20;
const MAP_INTERVAL: Duration = Duration::from_millis(50);

pub(super) struct Placement {
    runtime: Handle,
    task: Option<JoinHandle<()>>,
    compact: Option<bool>,
}

impl Placement {
    pub fn new(runtime: &Handle) -> Self {
        Self {
            runtime: runtime.clone(),
            task: None,
            compact: None,
        }
    }

    pub fn update(&mut self, compact: bool, size: Size<Pixels>) {
        if std::env::var_os("NIRI_SOCKET").is_none() {
            return;
        }
        if let Some(task) = self.task.take() {
            task.abort();
        }
        let width = f32::from(size.width).round() as u32;
        let height = f32::from(size.height).round() as u32;
        let center = self.compact != Some(compact);
        self.compact = Some(compact);
        self.task = Some(self.runtime.spawn(async move {
            if let Err(error) = place(compact, width, height, center).await {
                tracing::warn!(%error, "compositor could not place the BASR window");
            }
        }));
    }
}

impl Drop for Placement {
    fn drop(&mut self) {
        if let Some(task) = &self.task {
            task.abort();
        }
    }
}

/// # Errors
/// Reports unavailable Niri IPC or a rejected command. Only this process's BASR window is targeted.
async fn place(compact: bool, width: u32, height: u32, center: bool) -> Result<()> {
    for _ in 0..MAP_ATTEMPTS {
        let output = command(&["--json", "windows"]).await?;
        let windows: serde_json::Value = serde_json::from_slice(&output)?;
        let own_window = windows.as_array().and_then(|windows| {
            windows.iter().find(|window| {
                window["pid"].as_u64() == Some(u64::from(std::process::id()))
                    && window["app_id"].as_str() == Some("org.basr.Gui")
            })
        });
        if let Some(id) = own_window.and_then(|window| window["id"].as_u64()) {
            let id = id.to_string();
            if compact {
                command(&["action", "move-window-to-floating", "--id", &id]).await?;
                command(&[
                    "action",
                    "set-window-width",
                    "--id",
                    &id,
                    &width.to_string(),
                ])
                .await?;
                command(&[
                    "action",
                    "set-window-height",
                    "--id",
                    &id,
                    &height.to_string(),
                ])
                .await?;
            } else {
                command(&["action", "move-window-to-tiling", "--id", &id]).await?;
                command(&[
                    "action",
                    "set-window-width",
                    "--id",
                    &id,
                    &width.to_string(),
                ])
                .await?;
            }
            if center {
                command(&["action", "center-window", "--id", &id]).await?;
            }
            return Ok(());
        }
        tokio::time::sleep(MAP_INTERVAL).await;
    }
    anyhow::bail!("BASR window has not appeared in the compositor")
}

/// # Errors
/// Reports a command timeout, launch error, or nonzero exit status.
async fn command(args: &[&str]) -> Result<Vec<u8>> {
    let output = tokio::time::timeout(
        COMMAND_TIMEOUT,
        Command::new("niri")
            .arg("msg")
            .args(args)
            .kill_on_drop(true)
            .output(),
    )
    .await
    .context("Niri IPC timed out")??;
    anyhow::ensure!(output.status.success(), "Niri IPC command failed");
    Ok(output.stdout)
}
