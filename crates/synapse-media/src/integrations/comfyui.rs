//! ComfyUI HTTP integration — submit workflow, poll result, return output image path.

use anyhow::{Context, Result};
use serde_json::Value;
use std::time::Duration;

const DEFAULT_HOST: &str = "http://127.0.0.1:8188";

pub struct ComfyUi {
    host: String,
    client: reqwest::blocking::Client,
}

impl ComfyUi {
    pub fn new(host: Option<&str>) -> Self {
        Self {
            host: host.unwrap_or(DEFAULT_HOST).to_string(),
            client: reqwest::blocking::Client::builder()
                .timeout(Duration::from_secs(30))
                .build()
                .unwrap(),
        }
    }

    /// Check if ComfyUI is reachable.
    pub fn health(&self) -> bool {
        self.client
            .get(format!("{}/system_stats", self.host))
            .send()
            .map(|r| r.status().is_success())
            .unwrap_or(false)
    }

    /// Submit a workflow JSON, return prompt_id.
    pub fn submit_workflow(&self, workflow: Value) -> Result<String> {
        let body = serde_json::json!({ "prompt": workflow });
        let resp = self.client
            .post(format!("{}/prompt", self.host))
            .json(&body)
            .send()
            .context("ComfyUI POST /prompt")?;
        let json: Value = resp.json()?;
        let id = json["prompt_id"]
            .as_str()
            .context("no prompt_id in response")?
            .to_string();
        Ok(id)
    }

    /// Poll until done, return list of output image filenames.
    pub fn poll_result(&self, prompt_id: &str, timeout_secs: u64) -> Result<Vec<String>> {
        let deadline = std::time::Instant::now() + Duration::from_secs(timeout_secs);
        loop {
            if std::time::Instant::now() > deadline {
                anyhow::bail!("ComfyUI poll timeout after {timeout_secs}s");
            }
            let resp = self.client
                .get(format!("{}/history/{}", self.host, prompt_id))
                .send()
                .context("poll history")?;
            let json: Value = resp.json()?;
            if let Some(entry) = json.get(prompt_id) {
                let outputs = &entry["outputs"];
                let mut files = Vec::new();
                if let Some(obj) = outputs.as_object() {
                    for node in obj.values() {
                        if let Some(images) = node["images"].as_array() {
                            for img in images {
                                if let Some(fname) = img["filename"].as_str() {
                                    files.push(fname.to_string());
                                }
                            }
                        }
                    }
                }
                if !files.is_empty() {
                    return Ok(files);
                }
            }
            std::thread::sleep(Duration::from_millis(500));
        }
    }
}
