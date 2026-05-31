//! Remotion render-cli integration.
//! Generates props.json, calls `npx remotion render`, returns output mp4 path.

use anyhow::{Context, Result};
use serde_json::Value;
use std::path::{Path, PathBuf};
use std::process::Command;

pub struct RemotionRenderer {
    /// Path to Remotion project root (must have package.json + compositions).
    pub project_dir: String,
}

impl RemotionRenderer {
    pub fn new(project_dir: &str) -> Self {
        Self {
            project_dir: project_dir.to_string(),
        }
    }

    /// Check if `remotion` CLI is available.
    pub fn available() -> bool {
        Command::new("which")
            .arg("remotion")
            .output()
            .map(|o| o.status.success())
            .unwrap_or(false)
            || Command::new("npx")
                .args(["remotion", "--version"])
                .output()
                .map(|o| o.status.success())
                .unwrap_or(false)
    }

    /// Render a named composition with given props, return output mp4 path.
    pub fn render(&self, composition: &str, props: Value, output: &str) -> Result<PathBuf> {
        let props_path = Path::new(&self.project_dir).join("synapse-props.json");
        std::fs::write(&props_path, serde_json::to_string_pretty(&props)?)
            .context("write props.json")?;

        let out = Command::new("npx")
            .args([
                "remotion",
                "render",
                composition,
                output,
                "--props",
                props_path.to_str().unwrap(),
            ])
            .current_dir(&self.project_dir)
            .output()
            .context("npx remotion render")?;

        let _ = std::fs::remove_file(&props_path);

        if !out.status.success() {
            anyhow::bail!("{}", String::from_utf8_lossy(&out.stderr));
        }
        Ok(PathBuf::from(output))
    }
}
