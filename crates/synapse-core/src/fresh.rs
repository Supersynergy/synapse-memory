//! Freshness Guard: project-local dependency context for coding agents.
//!
//! The guard prevents version slippage: it treats manifest/lockfile versions as
//! the source of truth, optionally compares them to registries, and emits a
//! compact context block with version-pinned docs URLs.

use anyhow::Result;
use rusqlite::{Connection, params};
use serde::{Deserialize, Serialize};
use serde_json::Value as JsonValue;
use std::collections::{HashMap, HashSet};
use std::fs;
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime, UNIX_EPOCH};
use toml::Value as TomlValue;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FreshMode {
    Prompt,
    Session,
}

impl FreshMode {
    pub fn parse(s: &str) -> Self {
        if s.eq_ignore_ascii_case("session") {
            Self::Session
        } else {
            Self::Prompt
        }
    }
}

#[derive(Debug, Clone)]
pub struct FreshOptions {
    pub max_deps: usize,
    pub max_registry: usize,
    pub max_manifests: usize,
    pub max_depth: usize,
    pub scan_subprojects: bool,
    pub ttl_sec: u64,
    pub negative_ttl_sec: u64,
    pub timeout_ms: u64,
    pub cache_db: Option<PathBuf>,
}

impl FreshOptions {
    pub fn from_env(mode: FreshMode) -> Self {
        Self {
            max_deps: env_usize("SYNAPSE_FRESH_MAX_DEPS", 8),
            max_registry: env_usize(
                "SYNAPSE_FRESH_MAX_REGISTRY",
                if mode == FreshMode::Session { 3 } else { 5 },
            ),
            max_manifests: env_usize("SYNAPSE_FRESH_MAX_MANIFESTS", 8),
            max_depth: env_usize("SYNAPSE_FRESH_MAX_DEPTH", 3),
            scan_subprojects: std::env::var("SYNAPSE_FRESH_SCAN_SUBPROJECTS")
                .map(|v| v == "1" || v.eq_ignore_ascii_case("true"))
                .unwrap_or(false),
            ttl_sec: env_u64("SYNAPSE_FRESH_TTL_SEC", 21_600),
            negative_ttl_sec: env_u64("SYNAPSE_FRESH_NEG_TTL_SEC", 60),
            timeout_ms: env_u64(
                "SYNAPSE_FRESH_TIMEOUT_MS",
                if mode == FreshMode::Session { 250 } else { 750 },
            ),
            cache_db: fresh_cache_path(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct Dependency {
    pub ecosystem: String,
    pub name: String,
    pub declared: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub resolved: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub manifest: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct RegistryInfo {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub latest: Option<String>,
    pub source: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub docs: Option<String>,
    pub cached: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct BrokenHint {
    pub name: String,
    pub reason: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct FreshDepStatus {
    pub ecosystem: String,
    pub name: String,
    pub local: String,
    pub declared: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub latest: Option<String>,
    pub status: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub docs: Option<String>,
    pub cached: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub manifest: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct FreshReport {
    pub project: String,
    pub ttl_sec: u64,
    pub rule: String,
    pub broken: Vec<BrokenHint>,
    pub deps: Vec<FreshDepStatus>,
}

pub fn freshness_needed(prompt: &str, mode: FreshMode) -> bool {
    if std::env::var("SYNAPSE_FRESH_CONTEXT").ok().as_deref() == Some("0") {
        return false;
    }
    if mode == FreshMode::Session {
        return std::env::var("SYNAPSE_FRESH_ON_SESSION")
            .map(|v| v != "0")
            .unwrap_or(true);
    }
    let lower = prompt.to_ascii_lowercase();
    [
        "latest",
        "current",
        "version",
        "versions",
        "upgrade",
        "update",
        "install",
        "package",
        "dependency",
        "api",
        "docs",
        "documentation",
        "framework",
        "library",
        "npm",
        "cargo",
        "crate",
        "pip",
        "pypi",
        "pyproject",
        "package.json",
        "cargo.toml",
        "requirements",
        "context7",
        "fresh",
        "neueste",
        "aktuell",
        "versionen",
        "paket",
        "abhängigkeit",
        "abhaengigkeit",
        "doku",
        "schnittstelle",
    ]
    .iter()
    .any(|needle| lower.contains(needle))
}

pub fn build_fresh_report(
    prompt: &str,
    mode: FreshMode,
    cwd: Option<&Path>,
    project: Option<&str>,
    opts: &FreshOptions,
) -> Result<Option<FreshReport>> {
    let mut cache = FreshCache::open(opts.cache_db.as_deref()).ok();
    build_fresh_report_with_resolver(prompt, mode, cwd, project, opts, |dep, opts| {
        fetch_registry_latest(dep, opts, cache.as_mut())
    })
}

pub fn build_fresh_report_with_resolver<F>(
    prompt: &str,
    mode: FreshMode,
    cwd: Option<&Path>,
    project: Option<&str>,
    opts: &FreshOptions,
    mut registry: F,
) -> Result<Option<FreshReport>>
where
    F: FnMut(&Dependency, &FreshOptions) -> RegistryInfo,
{
    if !freshness_needed(prompt, mode) {
        return Ok(None);
    }
    let deps = collect_project_deps(cwd, prompt, opts)?;
    if deps.is_empty() {
        return Ok(None);
    }
    let project_name = project
        .filter(|p| !p.trim().is_empty())
        .map(str::to_string)
        .or_else(|| {
            cwd.and_then(|p| p.file_name())
                .map(|s| s.to_string_lossy().to_string())
        })
        .unwrap_or_else(|| "project".to_string());
    let mut statuses = Vec::with_capacity(deps.len());
    for (idx, dep) in deps.into_iter().enumerate() {
        let info = if idx < opts.max_registry {
            registry(&dep, opts)
        } else {
            RegistryInfo {
                latest: None,
                source: "not_checked".to_string(),
                docs: docs_url(&dep.ecosystem, &dep.name, None),
                cached: false,
            }
        };
        let local = dep.resolved.clone().unwrap_or_else(|| dep.declared.clone());
        let docs = docs_url(&dep.ecosystem, &dep.name, Some(&local)).or(info.docs);
        statuses.push(FreshDepStatus {
            ecosystem: dep.ecosystem,
            name: dep.name,
            local: local.clone(),
            declared: dep.declared,
            latest: info.latest.clone(),
            status: version_slip(&local, info.latest.as_deref()),
            docs,
            cached: info.cached,
            manifest: dep.manifest,
        });
    }
    Ok(Some(FreshReport {
        project: project_name,
        ttl_sec: opts.ttl_sec,
        rule: "code against the resolved/local version unless explicitly upgrading; if registry latest differs, avoid using APIs that only exist in the latest release.".to_string(),
        broken: broken_hints(prompt),
        deps: statuses,
    }))
}

pub fn render_fresh_context_xml(report: &FreshReport) -> String {
    let mut lines = vec![
        format!(
            "<fresh_context class=\"version_guard\" project=\"{}\" ttl_sec=\"{}\">",
            xml_escape(&report.project),
            report.ttl_sec
        ),
        format!("- Rule: {}", report.rule),
    ];
    for hint in &report.broken {
        lines.push(format!(
            "- broken/avoid {}: {}",
            hint.name,
            xml_escape(&hint.reason)
        ));
    }
    for dep in &report.deps {
        let latest = dep.latest.as_deref().unwrap_or("unknown");
        let cached = if dep.cached { " cached" } else { "" };
        let docs = dep
            .docs
            .as_ref()
            .map(|u| format!(" docs={}", xml_escape(u)))
            .unwrap_or_default();
        lines.push(format!(
            "- {}:{} local={} declared={} latest={} status={}{}{}",
            dep.ecosystem, dep.name, dep.local, dep.declared, latest, dep.status, cached, docs
        ));
    }
    lines.push("</fresh_context>".to_string());
    lines.join("\n")
}

pub fn collect_project_deps(
    cwd: Option<&Path>,
    prompt: &str,
    opts: &FreshOptions,
) -> Result<Vec<Dependency>> {
    let root = project_root_from_cwd(cwd)?;
    let mut deps = Vec::new();
    for manifest in nearest_manifests(&root, opts) {
        match manifest.file_name().and_then(|s| s.to_str()) {
            Some("Cargo.toml") => deps.extend(parse_cargo_manifest(&manifest)),
            Some("package.json") => deps.extend(parse_package_json(&manifest)),
            Some("pyproject.toml") => deps.extend(parse_pyproject(&manifest)),
            Some(name) if name.starts_with("requirements") => {
                deps.extend(parse_requirements(&manifest))
            }
            _ => {}
        }
    }

    let mut resolved: HashMap<(String, String), String> = HashMap::new();
    for (lock, ecosystem) in [
        ("Cargo.lock", "crates"),
        ("package-lock.json", "npm"),
        ("uv.lock", "pypi"),
    ] {
        let p = root.join(lock);
        if !p.exists() {
            continue;
        }
        let rows = match lock {
            "Cargo.lock" | "uv.lock" => parse_package_toml_lock(&p),
            "package-lock.json" => parse_package_lock(&p),
            _ => HashMap::new(),
        };
        for (name, version) in rows {
            resolved.insert((ecosystem.to_string(), name.to_ascii_lowercase()), version);
        }
    }

    let mut out = Vec::new();
    let mut seen = HashSet::new();
    let terms = query_terms(prompt, 30);
    for mut dep in deps {
        let key = (dep.ecosystem.clone(), dep.name.to_ascii_lowercase());
        if !seen.insert(key.clone()) {
            continue;
        }
        dep.resolved = resolved.get(&key).cloned();
        out.push(dep);
    }
    if !terms.is_empty() {
        let selected: Vec<_> = out
            .iter()
            .filter(|dep| {
                let name = dep.name.to_ascii_lowercase();
                terms.contains(&name)
                    || name
                        .split(['-', '_', '/', '@', '.'])
                        .filter(|p| !p.is_empty())
                        .any(|part| terms.contains(part))
            })
            .cloned()
            .collect();
        if !selected.is_empty() {
            return Ok(selected.into_iter().take(opts.max_deps).collect());
        }
    }
    Ok(out.into_iter().take(opts.max_deps).collect())
}

pub fn docs_url(ecosystem: &str, name: &str, version: Option<&str>) -> Option<String> {
    let pinned = version.and_then(exact_version);
    match ecosystem {
        "crates" => Some(if let Some(v) = pinned {
            format!("https://docs.rs/{}/{}/", name, v)
        } else {
            format!("https://docs.rs/{}/latest/", name)
        }),
        "npm" => {
            let pkg = percent_encode(name, true);
            Some(if let Some(v) = pinned {
                format!("https://www.npmjs.com/package/{}/v/{}", pkg, v)
            } else {
                format!("https://www.npmjs.com/package/{}", pkg)
            })
        }
        "pypi" => {
            let pkg = percent_encode(name, false);
            Some(if let Some(v) = pinned {
                format!("https://pypi.org/project/{}/{}/", pkg, v)
            } else {
                format!("https://pypi.org/project/{}/", pkg)
            })
        }
        _ => None,
    }
}

pub fn version_slip(local: &str, latest: Option<&str>) -> String {
    let Some(latest) = latest else {
        return "latest_unknown".to_string();
    };
    if exact_version(local).is_some() && local.trim() != latest {
        return "pinned_differs".to_string();
    }
    "ok_or_range".to_string()
}

fn fetch_registry_latest(
    dep: &Dependency,
    opts: &FreshOptions,
    cache: Option<&mut FreshCache>,
) -> RegistryInfo {
    let docs = docs_url(&dep.ecosystem, &dep.name, None);
    let now = now_secs();
    if let Some(cache) = cache {
        if let Ok(Some(info)) = cache.get(&dep.ecosystem, &dep.name, opts, now, docs.clone()) {
            return info;
        }
        let fetched = fetch_registry_uncached(&dep.ecosystem, &dep.name, opts, docs.clone());
        let _ = cache.put(&dep.ecosystem, &dep.name, &fetched, now);
        return fetched;
    }
    fetch_registry_uncached(&dep.ecosystem, &dep.name, opts, docs)
}

#[cfg(feature = "fresh-registry")]
fn fetch_registry_uncached(
    ecosystem: &str,
    name: &str,
    opts: &FreshOptions,
    docs: Option<String>,
) -> RegistryInfo {
    let url = registry_url(ecosystem, name);
    let latest = reqwest::blocking::Client::builder()
        .timeout(Duration::from_millis(opts.timeout_ms))
        .user_agent("synapse-fresh-context/1.0")
        .build()
        .ok()
        .and_then(|client| client.get(&url).send().ok())
        .and_then(|resp| resp.error_for_status().ok())
        .and_then(|resp| resp.json::<JsonValue>().ok())
        .and_then(|data| match ecosystem {
            "crates" => data.get("crate").and_then(|c| {
                c.get("max_stable_version")
                    .or_else(|| c.get("newest_version"))
                    .or_else(|| c.get("max_version"))
                    .and_then(|v| v.as_str())
                    .map(str::to_string)
            }),
            "npm" => data
                .get("version")
                .and_then(|v| v.as_str())
                .map(str::to_string),
            "pypi" => data
                .get("info")
                .and_then(|i| i.get("version"))
                .and_then(|v| v.as_str())
                .map(str::to_string),
            _ => None,
        });
    RegistryInfo {
        latest,
        source: url,
        docs,
        cached: false,
    }
}

#[cfg(not(feature = "fresh-registry"))]
fn fetch_registry_uncached(
    ecosystem: &str,
    name: &str,
    _opts: &FreshOptions,
    docs: Option<String>,
) -> RegistryInfo {
    RegistryInfo {
        latest: None,
        source: registry_url(ecosystem, name),
        docs,
        cached: false,
    }
}

fn registry_url(ecosystem: &str, name: &str) -> String {
    match ecosystem {
        "crates" => format!(
            "https://crates.io/api/v1/crates/{}",
            percent_encode(name, false)
        ),
        "npm" => format!(
            "https://registry.npmjs.org/{}/latest",
            percent_encode(name, false)
        ),
        "pypi" => format!("https://pypi.org/pypi/{}/json", percent_encode(name, false)),
        _ => String::new(),
    }
}

struct FreshCache {
    conn: Connection,
}

impl FreshCache {
    fn open(path: Option<&Path>) -> Result<Self> {
        let Some(path) = path else {
            anyhow::bail!("no fresh cache path");
        };
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).ok();
        }
        let conn = Connection::open(path)?;
        conn.execute_batch(
            r#"
            PRAGMA journal_mode=WAL;
            PRAGMA synchronous=NORMAL;
            CREATE TABLE IF NOT EXISTS registry_cache (
                ecosystem TEXT NOT NULL,
                name TEXT NOT NULL,
                latest TEXT,
                source TEXT NOT NULL,
                docs TEXT,
                ts INTEGER NOT NULL,
                PRIMARY KEY(ecosystem, name)
            );
            "#,
        )?;
        Ok(Self { conn })
    }

    fn get(
        &mut self,
        ecosystem: &str,
        name: &str,
        opts: &FreshOptions,
        now: u64,
        docs: Option<String>,
    ) -> Result<Option<RegistryInfo>> {
        let mut stmt = self.conn.prepare(
            "SELECT latest, source, docs, ts FROM registry_cache WHERE ecosystem=?1 AND name=?2",
        )?;
        let row = stmt.query_row(params![ecosystem, name.to_ascii_lowercase()], |r| {
            Ok((
                r.get::<_, Option<String>>(0)?,
                r.get::<_, String>(1)?,
                r.get::<_, Option<String>>(2)?,
                r.get::<_, i64>(3)?,
            ))
        });
        let Ok((latest, source, cached_docs, ts)) = row else {
            return Ok(None);
        };
        let ttl = if latest.is_some() {
            opts.ttl_sec
        } else {
            opts.negative_ttl_sec
        };
        if now.saturating_sub(ts.max(0) as u64) < ttl {
            return Ok(Some(RegistryInfo {
                latest,
                source,
                docs: cached_docs.or(docs),
                cached: true,
            }));
        }
        Ok(None)
    }

    fn put(&mut self, ecosystem: &str, name: &str, info: &RegistryInfo, now: u64) -> Result<()> {
        self.conn.execute(
            r#"
            INSERT OR REPLACE INTO registry_cache(ecosystem, name, latest, source, docs, ts)
            VALUES(?1, ?2, ?3, ?4, ?5, ?6)
            "#,
            params![
                ecosystem,
                name.to_ascii_lowercase(),
                info.latest,
                info.source,
                info.docs,
                now as i64
            ],
        )?;
        Ok(())
    }
}

fn project_root_from_cwd(cwd: Option<&Path>) -> Result<PathBuf> {
    let mut start = cwd
        .map(Path::to_path_buf)
        .unwrap_or(std::env::current_dir()?)
        .canonicalize()
        .unwrap_or_else(|_| cwd.unwrap_or_else(|| Path::new(".")).to_path_buf());
    if start.is_file() {
        start = start.parent().unwrap_or(Path::new(".")).to_path_buf();
    }
    let markers = [
        "Cargo.toml",
        "package.json",
        "pyproject.toml",
        "requirements.txt",
        ".git",
    ];
    let mut cur = start.clone();
    loop {
        if markers.iter().any(|m| cur.join(m).exists()) {
            return Ok(cur);
        }
        if !cur.pop() {
            return Ok(start);
        }
    }
}

fn nearest_manifests(root: &Path, opts: &FreshOptions) -> Vec<PathBuf> {
    let names = [
        "Cargo.toml",
        "package.json",
        "pyproject.toml",
        "requirements.txt",
    ];
    let mut found = Vec::new();
    for name in names {
        let p = root.join(name);
        if p.exists() {
            found.push(p);
        }
    }
    if !found.is_empty() && !opts.scan_subprojects {
        found.truncate(opts.max_manifests);
        return found;
    }
    if found.len() >= opts.max_manifests {
        found.truncate(opts.max_manifests);
        return found;
    }
    let mut seen: HashSet<PathBuf> = found.iter().cloned().collect();
    walk_manifests(root, root, opts, &mut found, &mut seen);
    found.truncate(opts.max_manifests);
    found
}

fn walk_manifests(
    root: &Path,
    cur: &Path,
    opts: &FreshOptions,
    found: &mut Vec<PathBuf>,
    seen: &mut HashSet<PathBuf>,
) {
    if found.len() >= opts.max_manifests {
        return;
    }
    let depth = cur
        .strip_prefix(root)
        .map(|p| p.components().count())
        .unwrap_or(0);
    if depth > opts.max_depth {
        return;
    }
    let Ok(entries) = fs::read_dir(cur) else {
        return;
    };
    let skip = [
        "node_modules",
        "target",
        ".git",
        "__pycache__",
        ".venv",
        "dist",
        "build",
        ".next",
    ];
    for entry in entries.flatten() {
        let path = entry.path();
        let name = entry.file_name().to_string_lossy().to_string();
        if path.is_dir() {
            if !skip.contains(&name.as_str()) {
                walk_manifests(root, &path, opts, found, seen);
            }
            continue;
        }
        if matches!(
            name.as_str(),
            "Cargo.toml" | "package.json" | "pyproject.toml" | "requirements.txt"
        ) && seen.insert(path.clone())
        {
            found.push(path);
            if found.len() >= opts.max_manifests {
                return;
            }
        }
    }
}

fn parse_cargo_manifest(path: &Path) -> Vec<Dependency> {
    let Ok(text) = fs::read_to_string(path) else {
        return vec![];
    };
    let Ok(data) = toml::from_str::<TomlValue>(&text) else {
        return vec![];
    };
    let mut deps = Vec::new();
    for section in ["dependencies", "dev-dependencies", "build-dependencies"] {
        if let Some(table) = data.get(section).and_then(|v| v.as_table()) {
            deps.extend(
                table
                    .iter()
                    .map(|(name, spec)| dep("crates", name, parse_dep_spec(spec), path)),
            );
        }
    }
    if let Some(table) = data
        .get("workspace")
        .and_then(|v| v.get("dependencies"))
        .and_then(|v| v.as_table())
    {
        deps.extend(
            table
                .iter()
                .map(|(name, spec)| dep("crates", name, parse_dep_spec(spec), path)),
        );
    }
    deps
}

fn parse_package_json(path: &Path) -> Vec<Dependency> {
    let Ok(text) = fs::read_to_string(path) else {
        return vec![];
    };
    let Ok(data) = serde_json::from_str::<JsonValue>(&text) else {
        return vec![];
    };
    let mut deps = Vec::new();
    for section in [
        "dependencies",
        "devDependencies",
        "peerDependencies",
        "optionalDependencies",
    ] {
        if let Some(obj) = data.get(section).and_then(|v| v.as_object()) {
            deps.extend(obj.iter().map(|(name, spec)| {
                dep(
                    "npm",
                    name,
                    spec.as_str()
                        .map(str::to_string)
                        .unwrap_or_else(|| spec.to_string()),
                    path,
                )
            }));
        }
    }
    deps
}

fn parse_pyproject(path: &Path) -> Vec<Dependency> {
    let Ok(text) = fs::read_to_string(path) else {
        return vec![];
    };
    let Ok(data) = toml::from_str::<TomlValue>(&text) else {
        return vec![];
    };
    let mut deps = Vec::new();
    if let Some(arr) = data
        .get("project")
        .and_then(|v| v.get("dependencies"))
        .and_then(|v| v.as_array())
    {
        for spec in arr.iter().filter_map(|v| v.as_str()) {
            if let Some((name, declared)) = pep_dep_name(spec) {
                deps.push(dep("pypi", &name, declared, path));
            }
        }
    }
    if let Some(optional) = data
        .get("project")
        .and_then(|v| v.get("optional-dependencies"))
        .and_then(|v| v.as_table())
    {
        for arr in optional.values().filter_map(|v| v.as_array()) {
            for spec in arr.iter().filter_map(|v| v.as_str()) {
                if let Some((name, declared)) = pep_dep_name(spec) {
                    deps.push(dep("pypi", &name, declared, path));
                }
            }
        }
    }
    if let Some(poetry) = data
        .get("tool")
        .and_then(|v| v.get("poetry"))
        .and_then(|v| v.get("dependencies"))
        .and_then(|v| v.as_table())
    {
        for (name, spec) in poetry {
            if !name.eq_ignore_ascii_case("python") {
                deps.push(dep("pypi", name, parse_dep_spec(spec), path));
            }
        }
    }
    deps
}

fn parse_requirements(path: &Path) -> Vec<Dependency> {
    let Ok(text) = fs::read_to_string(path) else {
        return vec![];
    };
    text.lines()
        .filter_map(pep_dep_name)
        .map(|(name, declared)| dep("pypi", &name, declared, path))
        .collect()
}

fn parse_dep_spec(v: &TomlValue) -> String {
    if let Some(s) = v.as_str() {
        return s.to_string();
    }
    if let Some(table) = v.as_table() {
        if let Some(version) = table.get("version").and_then(|v| v.as_str()) {
            return version.to_string();
        }
        if let Some(path) = table.get("path").and_then(|v| v.as_str()) {
            return format!("path:{path}");
        }
        if let Some(git) = table.get("git").and_then(|v| v.as_str()) {
            return format!("git:{git}");
        }
    }
    v.to_string()
}

fn parse_package_toml_lock(path: &Path) -> HashMap<String, String> {
    let Ok(text) = fs::read_to_string(path) else {
        return HashMap::new();
    };
    let mut out = HashMap::new();
    let mut name: Option<String> = None;
    let mut version: Option<String> = None;
    for raw in text.lines() {
        let line = raw.trim();
        if line == "[[package]]" {
            flush_lock_pkg(&mut out, &mut name, &mut version);
        } else if let Some(rest) = line.strip_prefix("name = ") {
            name = Some(rest.trim().trim_matches('"').to_string());
        } else if let Some(rest) = line.strip_prefix("version = ") {
            version = Some(rest.trim().trim_matches('"').to_string());
        }
    }
    flush_lock_pkg(&mut out, &mut name, &mut version);
    out
}

fn parse_package_lock(path: &Path) -> HashMap<String, String> {
    let Ok(text) = fs::read_to_string(path) else {
        return HashMap::new();
    };
    let Ok(data) = serde_json::from_str::<JsonValue>(&text) else {
        return HashMap::new();
    };
    let mut out = HashMap::new();
    if let Some(packages) = data.get("packages").and_then(|v| v.as_object()) {
        for (loc, meta) in packages {
            if let Some(name) = loc.strip_prefix("node_modules/")
                && let Some(version) = meta.get("version").and_then(|v| v.as_str())
            {
                out.insert(name.to_string(), version.to_string());
            }
        }
    }
    if let Some(deps) = data.get("dependencies").and_then(|v| v.as_object()) {
        for (name, meta) in deps {
            if let Some(version) = meta.get("version").and_then(|v| v.as_str()) {
                out.entry(name.to_string())
                    .or_insert_with(|| version.to_string());
            }
        }
    }
    out
}

fn flush_lock_pkg(
    out: &mut HashMap<String, String>,
    name: &mut Option<String>,
    version: &mut Option<String>,
) {
    if let (Some(n), Some(v)) = (name.take(), version.take()) {
        out.insert(n, v);
    }
}

fn pep_dep_name(spec: &str) -> Option<(String, String)> {
    let spec = spec.trim();
    if spec.is_empty() || spec.starts_with('#') || spec.starts_with('-') {
        return None;
    }
    let mut end = 0;
    for ch in spec.chars() {
        if ch.is_ascii_alphanumeric() || matches!(ch, '_' | '.' | '-') {
            end += ch.len_utf8();
        } else {
            break;
        }
    }
    if end == 0 {
        return None;
    }
    let name = spec[..end].to_string();
    let declared = spec[end..].trim();
    Some((
        name,
        if declared.is_empty() { "*" } else { declared }.to_string(),
    ))
}

fn dep(ecosystem: &str, name: &str, declared: String, path: &Path) -> Dependency {
    Dependency {
        ecosystem: ecosystem.to_string(),
        name: name.to_string(),
        declared,
        resolved: None,
        manifest: Some(path.display().to_string()),
    }
}

fn exact_version(value: &str) -> Option<&str> {
    let s = value.trim();
    if s.is_empty() || !s.chars().next().is_some_and(|c| c.is_ascii_digit()) {
        return None;
    }
    if s.chars()
        .all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '-'))
    {
        Some(s)
    } else {
        None
    }
}

fn broken_hints(prompt: &str) -> Vec<BrokenHint> {
    [
        (
            "chromadb",
            "avoid: broken/reliability issues locally; prefer Qdrant or LanceDB",
        ),
        (
            "chroma",
            "avoid for new local agent memory unless explicitly testing Chroma; prefer Qdrant/LanceDB/Synapse",
        ),
        ("weaviate", "avoid: heavy single-node footprint here"),
        (
            "langchain",
            "avoid: deprecated local preference; prefer direct SDKs, DSPy/LangGraph only when needed",
        ),
        (
            "selenium",
            "avoid: legacy browser automation; prefer Playwright/Patchwright/nodriver",
        ),
        ("smollm2", "avoid for instruction-critical extraction in this environment"),
    ]
    .into_iter()
    .filter(|(name, _)| contains_name(prompt, name))
    .map(|(name, reason)| BrokenHint {
        name: name.to_string(),
        reason: reason.to_string(),
    })
    .collect()
}

fn contains_name(text: &str, needle: &str) -> bool {
    text.split(|c: char| !(c.is_ascii_alphanumeric() || matches!(c, '_' | '-' | '.')))
        .any(|part| part.eq_ignore_ascii_case(needle))
}

fn query_terms(text: &str, limit: usize) -> HashSet<String> {
    let stop = [
        "the", "and", "for", "with", "bitte", "mache", "mach", "jetzt", "latest", "version", "api",
        "docs", "neueste", "aktuell",
    ];
    let mut out = HashSet::new();
    for term in text
        .split(|c: char| !(c.is_ascii_alphanumeric() || matches!(c, '_' | '.' | '/' | ':' | '-')))
        .map(|s| s.trim_matches(|c: char| ".,;:!?\"'()[]{}<>".contains(c)))
        .filter(|s| s.len() >= 3)
    {
        let lower = term.to_ascii_lowercase();
        if stop.contains(&lower.as_str()) {
            continue;
        }
        out.insert(lower);
        if out.len() >= limit {
            break;
        }
    }
    out
}

fn percent_encode(input: &str, keep_slash_at: bool) -> String {
    let mut out = String::with_capacity(input.len());
    for b in input.bytes() {
        let keep = b.is_ascii_alphanumeric()
            || matches!(b, b'-' | b'.' | b'_' | b'~')
            || (keep_slash_at && matches!(b, b'/' | b'@'));
        if keep {
            out.push(b as char);
        } else {
            out.push_str(&format!("%{b:02X}"));
        }
    }
    out
}

fn xml_escape(input: &str) -> String {
    input
        .replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
}

fn fresh_cache_path() -> Option<PathBuf> {
    if let Some(explicit) = std::env::var_os("SYNAPSE_FRESH_CONTEXT_DB") {
        return Some(PathBuf::from(explicit));
    }
    std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".synapse/fresh_context.db"))
}

fn env_usize(name: &str, default: usize) -> usize {
    std::env::var(name)
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(default)
}

fn env_u64(name: &str, default: u64) -> u64 {
    std::env::var(name)
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(default)
}

fn now_secs() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or(Duration::ZERO)
        .as_secs()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Mutex;
    use tempfile::tempdir;

    static ENV_LOCK: Mutex<()> = Mutex::new(());

    fn opts(cache: &Path) -> FreshOptions {
        FreshOptions {
            max_deps: 8,
            max_registry: 5,
            max_manifests: 8,
            max_depth: 3,
            scan_subprojects: false,
            ttl_sec: 21_600,
            negative_ttl_sec: 300,
            timeout_ms: 1,
            cache_db: Some(cache.to_path_buf()),
        }
    }

    #[test]
    fn package_manifest_marks_slip_and_broken_hint() {
        let _guard = ENV_LOCK.lock().unwrap();
        unsafe { std::env::remove_var("SYNAPSE_FRESH_CONTEXT") };
        let td = tempdir().unwrap();
        fs::write(
            td.path().join("package.json"),
            r#"{"dependencies":{"left-pad":"1.0.0","langchain":"0.3.0"}}"#,
        )
        .unwrap();
        let report = build_fresh_report_with_resolver(
            "latest left-pad langchain",
            FreshMode::Prompt,
            Some(td.path()),
            Some("proj"),
            &opts(&td.path().join("fresh.db")),
            |_dep, _opts| RegistryInfo {
                latest: Some("1.3.0".to_string()),
                source: "test".to_string(),
                docs: None,
                cached: false,
            },
        )
        .unwrap()
        .unwrap();
        let xml = render_fresh_context_xml(&report);
        assert!(xml.contains("npm:left-pad"));
        assert!(xml.contains("latest=1.3.0"));
        assert!(xml.contains("status=pinned_differs"));
        assert!(xml.contains("docs=https://www.npmjs.com/package/left-pad/v/1.0.0"));
        assert!(xml.contains("broken/avoid langchain"));
    }

    #[test]
    fn cargo_lock_resolution_pins_docs_to_local_version() {
        let _guard = ENV_LOCK.lock().unwrap();
        unsafe { std::env::remove_var("SYNAPSE_FRESH_CONTEXT") };
        let td = tempdir().unwrap();
        fs::write(
            td.path().join("Cargo.toml"),
            r#"
[package]
name = "x"
version = "0.1.0"
edition = "2021"

[dependencies]
serde = "1"
"#,
        )
        .unwrap();
        fs::write(
            td.path().join("Cargo.lock"),
            r#"
[[package]]
name = "serde"
version = "1.0.228"
"#,
        )
        .unwrap();
        assert!(freshness_needed("latest serde api", FreshMode::Prompt));
        let manifest_deps = parse_cargo_manifest(&td.path().join("Cargo.toml"));
        assert_eq!(manifest_deps.len(), 1, "manifest_deps: {manifest_deps:?}");
        let deps = collect_project_deps(
            Some(td.path()),
            "latest serde api",
            &opts(&td.path().join("fresh.db")),
        )
        .unwrap();
        assert_eq!(deps.len(), 1, "deps: {deps:?}");
        let report = build_fresh_report_with_resolver(
            "latest serde api",
            FreshMode::Prompt,
            Some(td.path()),
            Some("proj"),
            &opts(&td.path().join("fresh.db")),
            |_dep, _opts| RegistryInfo {
                latest: Some("1.0.228".to_string()),
                source: "test".to_string(),
                docs: Some("https://docs.rs/serde/latest/".to_string()),
                cached: false,
            },
        )
        .unwrap()
        .unwrap();
        let dep = &report.deps[0];
        assert_eq!(dep.local, "1.0.228");
        assert_eq!(dep.status, "ok_or_range");
        assert_eq!(dep.docs.as_deref(), Some("https://docs.rs/serde/1.0.228/"));
    }

    #[test]
    fn disabled_by_env() {
        let _guard = ENV_LOCK.lock().unwrap();
        unsafe { std::env::set_var("SYNAPSE_FRESH_CONTEXT", "0") };
        assert!(!freshness_needed("latest serde", FreshMode::Prompt));
        unsafe { std::env::remove_var("SYNAPSE_FRESH_CONTEXT") };
    }
}
