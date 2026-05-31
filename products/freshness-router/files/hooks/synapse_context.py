#!/usr/bin/env python3
"""Budgeted Synapse context injection for Claude Code hooks.

The hook is intentionally stdlib-only and fail-open: if Synapse is unavailable,
it exits quietly. The policy is a tiny SuperML-style router: classify the prompt,
fetch a wider candidate set, then pack only diverse, query-centered snippets into
a hard token budget.
"""

from __future__ import annotations

import argparse
import hashlib
import json
import os
import re
import shutil
import sqlite3
import subprocess
import sys
import tomllib
import urllib.error
import urllib.parse
import urllib.request
from contextlib import contextmanager, nullcontext
from dataclasses import dataclass, field
from pathlib import Path
from time import perf_counter_ns, time


STOPWORDS = {
    "a",
    "an",
    "and",
    "are",
    "as",
    "auf",
    "aus",
    "be",
    "bei",
    "bitte",
    "can",
    "check",
    "da",
    "das",
    "der",
    "die",
    "do",
    "du",
    "ein",
    "eine",
    "er",
    "es",
    "fix",
    "for",
    "gibt",
    "hier",
    "how",
    "ich",
    "in",
    "is",
    "ist",
    "it",
    "jetzt",
    "mal",
    "me",
    "mit",
    "mache",
    "mach",
    "my",
    "nach",
    "noch",
    "of",
    "on",
    "or",
    "so",
    "that",
    "the",
    "this",
    "to",
    "und",
    "use",
    "von",
    "was",
    "wie",
    "wir",
    "with",
    "you",
}


@dataclass(frozen=True)
class RecallQuery:
    name: str
    query: str
    weight: float = 1.0


@dataclass(frozen=True)
class Policy:
    name: str
    query: str
    fetch_k: int
    max_tokens: int
    min_score: float
    snippet_chars: int
    max_items: int
    perspectives: tuple[RecallQuery, ...] = field(default_factory=tuple)


@dataclass
class Hit:
    score: float
    title: str
    text: str
    snippet: str
    adjusted: float
    tokens: int
    perspective: str = "primary"


@dataclass
class ContextBuild:
    text: str
    policy: Policy
    packed: list[Hit]
    used_tokens: int
    naive_tokens: int
    latency_ms: float
    event_id: str | None = None


@dataclass(frozen=True)
class Dep:
    ecosystem: str
    name: str
    declared: str
    resolved: str | None = None
    manifest: str | None = None


@dataclass(frozen=True)
class RegistryInfo:
    latest: str | None
    source: str
    docs: str | None = None
    cached: bool = False


def estimate_tokens(text: str) -> int:
    return max(1, (len(text) + 3) // 4)


def normalize_ws(text: str) -> str:
    return re.sub(r"\s+", " ", text).strip()


def parse_hook_input(raw: str) -> tuple[str, str | None, str | None]:
    """Return prompt, session_id, cwd from raw text or Claude Code hook JSON."""
    raw = raw.strip()
    if not raw:
        return "", None, None
    try:
        data = json.loads(raw)
    except Exception:
        return raw, None, None
    if not isinstance(data, dict):
        return raw, None, None
    prompt = data.get("prompt") or data.get("input") or data.get("message") or ""
    if isinstance(prompt, dict):
        prompt = json.dumps(prompt, ensure_ascii=False)
    return str(prompt), data.get("session_id") or data.get("sessionId"), data.get("cwd")


def query_terms(text: str, limit: int = 14) -> list[str]:
    terms: list[str] = []
    seen: set[str] = set()
    for term in re.findall(r"[A-Za-z0-9_./:-]{3,}", text.lower()):
        term = term.strip(".,;:!?\"'()[]{}<>")
        if len(term) < 3 or term in STOPWORDS or term in seen:
            continue
        seen.add(term)
        terms.append(term)
        if len(terms) >= limit:
            break
    return terms


def classify_prompt(prompt: str, mode: str, project: str | None) -> Policy | None:
    text = normalize_ws(prompt)
    lower = text.lower()
    env_budget = os.environ.get("SYNAPSE_CONTEXT_MAX_TOKENS")

    def scoped_query(max_chars: int) -> str:
        q = text[:max_chars]
        if project:
            proj = normalize_ws(project)
            if proj and proj.lower() not in q.lower():
                q = f"{q} {proj}"
        return q

    if mode == "session":
        proj = project or Path(os.getcwd()).name
        budget = int(env_budget or os.environ.get("SYNAPSE_SESSION_CONTEXT_TOKENS", "520"))
        query = f"{proj} decisions stack preferences bugs architecture verification recent"
        return Policy("session", query, 16, budget, 0.008, 260, 8)

    if len(text) < 20 or re.fullmatch(r"[\s+._!?-]*(ok|ja|yes|weiter|\+|go|done)?[\s+._!?-]*", lower):
        return None

    budget_default = int(env_budget or os.environ.get("SYNAPSE_PROMPT_CONTEXT_TOKENS", "420"))

    if re.search(r"traceback|exception|panic|failed|failing|error|bug|stacktrace|cargo|pytest|test|lint|build|crash", lower):
        return Policy("debug", scoped_query(500), 14, min(max(budget_default, 520), 760), 0.006, 300, 8)

    if re.search(r"autolearn|superml|optimi|benchmark|simulate|simulation|eval|recall|token|context|routing|rerank", lower):
        return Policy("optimize", scoped_query(560), 16, min(max(budget_default, 620), 840), 0.006, 320, 10)

    if re.search(r"remember|previous|last session|letzte|vorher|history|entscheidung|decision|warum", lower):
        return Policy("history", scoped_query(520), 18, min(max(budget_default, 680), 900), 0.005, 340, 10)

    if len(text) < 80:
        return Policy("compact", scoped_query(300), 8, min(budget_default, 280), 0.010, 220, 5)

    return Policy("default", scoped_query(420), 12, budget_default, 0.008, 260, 7)


def executable_exists(cmd: str) -> bool:
    return Path(cmd).exists() or shutil.which(cmd) is not None


def synx_fresh_bin() -> str | None:
    explicit = os.environ.get("SYNX_FRESH_BIN") or os.environ.get("SYNX_BIN") or os.environ.get("SYN_BIN")
    if explicit and executable_exists(explicit):
        return explicit
    return shutil.which("synx") or ("/Users/master/.local/bin/synx" if Path("/Users/master/.local/bin/synx").exists() else None)


def synx_fast_bin() -> str | None:
    explicit = os.environ.get("SYNX_FAST_BIN")
    if explicit and executable_exists(explicit):
        return explicit
    return (
        shutil.which("synx-fast")
        or ("/Users/master/.local/bin/synx-fast" if Path("/Users/master/.local/bin/synx-fast").exists() else None)
        or synx_fresh_bin()
    )


def synx_bin() -> str | None:
    return synx_fresh_bin()


def synx_fast_supports_scoped(binary: str) -> bool:
    return Path(binary).name == "synx-fast" or os.environ.get("SYNAPSE_CONTEXT_FORCE_SCOPED") == "1"


def scoped_cli_args(binary: str, project: str | None) -> list[str]:
    if not project or not synx_fast_supports_scoped(binary):
        return []
    args = ["--scope", project]
    scope_key = os.environ.get("SYNAPSE_SCOPE_KEY")
    if scope_key:
        args.extend(["--scope-key", scope_key])
    return args


def learning_enabled() -> bool:
    return os.environ.get("SYNAPSE_CONTEXT_NO_LEARN") != "1"


def learn_db_path() -> Path:
    explicit = os.environ.get("SYNAPSE_RECALL_LEARN_DB")
    if explicit:
        return Path(explicit).expanduser()
    return Path.home() / ".synapse" / "recall_learn.db"


def connect_learn_db() -> sqlite3.Connection:
    path = learn_db_path()
    path.parent.mkdir(parents=True, exist_ok=True)
    conn = sqlite3.connect(path, timeout=0.2)
    conn.execute("PRAGMA journal_mode=WAL")
    conn.execute("PRAGMA synchronous=NORMAL")
    conn.execute(
        """
        CREATE TABLE IF NOT EXISTS recall_events (
            id TEXT PRIMARY KEY,
            ts INTEGER NOT NULL,
            mode TEXT NOT NULL,
            policy TEXT NOT NULL,
            project TEXT,
            session_id TEXT,
            query_hash TEXT NOT NULL,
            perspectives TEXT NOT NULL,
            hits_json TEXT NOT NULL,
            tokens_est INTEGER NOT NULL,
            naive_tokens INTEGER NOT NULL,
            saved_ratio REAL NOT NULL,
            latency_ms REAL NOT NULL,
            reward REAL,
            reward_ts INTEGER
        )
        """
    )
    conn.execute(
        """
        CREATE TABLE IF NOT EXISTS perspective_bandit (
            policy TEXT NOT NULL,
            perspective TEXT NOT NULL,
            alpha REAL NOT NULL DEFAULT 1.0,
            beta REAL NOT NULL DEFAULT 1.0,
            uses INTEGER NOT NULL DEFAULT 0,
            rewards INTEGER NOT NULL DEFAULT 0,
            updated_ts INTEGER NOT NULL,
            PRIMARY KEY(policy, perspective)
        )
        """
    )
    return conn


@contextmanager
def open_learn_db():
    conn = connect_learn_db()
    try:
        yield conn
        conn.commit()
    finally:
        conn.close()


def stable_hash(text: str, n: int = 16) -> str:
    return hashlib.blake2b(text.encode("utf-8", "ignore"), digest_size=16).hexdigest()[:n]


def load_policy_bandit(policy_name: str) -> dict[str, tuple[float, float, int]]:
    if not learning_enabled():
        return {}
    try:
        with open_learn_db() as conn:
            rows = conn.execute(
                "SELECT perspective, alpha, beta, uses FROM perspective_bandit WHERE policy=?1",
                (policy_name,),
            ).fetchall()
    except Exception:
        return {}
    return {str(p): (float(a), float(b), int(u)) for p, a, b, u in rows}


def learned_weight(policy_name: str, perspective: str, base: float, bandit: dict[str, tuple[float, float, int]]) -> float:
    alpha, beta, uses = bandit.get(perspective, (1.0, 1.0, 0))
    mean = alpha / max(0.001, alpha + beta)
    confidence = min(1.0, uses / 20.0)
    multiplier = 1.0 + (mean - 0.5) * 0.22 * confidence
    return max(0.82, min(1.22, base * multiplier))


def recall_plan(policy: Policy) -> list[RecallQuery]:
    if policy.perspectives:
        return list(policy.perspectives)

    base = normalize_ws(policy.query)
    terms = " ".join(query_terms(base, 10))
    bandit = load_policy_bandit(policy.name)
    plan = [RecallQuery("primary", base, learned_weight(policy.name, "primary", 1.0, bandit))]

    def add(name: str, prefix: str, weight: float) -> None:
        q = normalize_ws(f"{prefix} {terms or base}")
        if q and all(existing.query != q for existing in plan):
            plan.append(RecallQuery(name, q, learned_weight(policy.name, name, weight, bandit)))

    if policy.name == "debug":
        add("fix", "verified fix failing test bug panic regression root cause", 1.12)
        add("decision", "decision why changed workaround caveat verification", 1.08)
        add("code", "implementation file function cargo pytest lint", 1.02)
    elif policy.name == "optimize":
        add("bench", "benchmark latency p50 p95 p99 recall token speedup verification", 1.14)
        add("decision", "decision architecture tradeoff bottleneck winning edge", 1.10)
        add("learn", "autolearn superml reward rerank routing eval outcome", 1.04)
        add("code", "implementation hook pipeline cache batch context", 1.02)
    elif policy.name == "history":
        add("decision", "decision rationale previous session verified outcome", 1.14)
        add("telepathy", "telepathy reply prompt tools recent project", 1.06)
        add("switch", "switched from using because caveat", 1.04)
    elif policy.name == "session":
        add("decision", "project decisions architecture verified bugs recent", 1.12)
        add("workflow", "hooks telepathy context benchmark verification", 1.06)
        add("risk", "known caveat broken avoid warning workaround", 1.02)
    elif policy.name == "compact":
        add("decision", "decision verified recent", 1.06)
    else:
        add("decision", "decision architecture verified prior work", 1.08)
        add("code", "implementation file test bug context", 1.02)

    cap = int(os.environ.get("SYNAPSE_CONTEXT_MAX_PERSPECTIVES", "5"))
    return plan[: max(1, cap)]


def _extract_response_hits(resp: object) -> list[dict]:
    if not isinstance(resp, dict):
        return []
    hits = resp.get("Hits")
    if hits is None:
        hits = resp.get("hits")
    if hits is None and isinstance(resp.get("Ok"), dict):
        hits = resp["Ok"].get("Hits") or resp["Ok"].get("hits")
    return hits if isinstance(hits, list) else []


def _batch_lines(stdout: str, plan: list[RecallQuery]) -> list[str]:
    by_query = {q.query: q for q in plan}
    lines: list[str] = []
    fallback_idx = 0
    for raw in stdout.splitlines():
        raw = raw.strip()
        if not raw:
            continue
        try:
            row = json.loads(raw)
        except Exception:
            continue
        query = str(row.get("q") or "")
        rq = by_query.get(query)
        if rq is None:
            rq = plan[min(fallback_idx, len(plan) - 1)]
            fallback_idx += 1
        for hit in _extract_response_hits(row.get("response")):
            if not isinstance(hit, dict):
                continue
            try:
                score = float(hit.get("score", 0.0)) * rq.weight
            except (TypeError, ValueError):
                score = 0.0
            title = normalize_ws(str(hit.get("title") or hit.get("uri") or f"doc:{hit.get('id', '')}"))
            text = normalize_ws(str(hit.get("text") or ""))
            if text:
                lines.append(f"{rq.name}\t{score:.9f}\t{title}\t{text}")
    return lines


def fetch_hits(policy: Policy, project: str | None = None) -> list[str]:
    binary = synx_fast_bin()
    if not binary:
        return []
    plan = recall_plan(policy)
    timeout = float(os.environ.get("SYNAPSE_CONTEXT_TIMEOUT", "1.2"))
    scope_args = scoped_cli_args(binary, project)
    if len(plan) > 1 and os.environ.get("SYNAPSE_CONTEXT_NO_BATCH") != "1":
        try:
            proc = subprocess.run(
                [binary, "batch", "hybrid", "--limit", str(policy.fetch_k), *scope_args],
                input="\n".join(q.query for q in plan) + "\n",
                capture_output=True,
                text=True,
                timeout=timeout,
            )
            if proc.returncode == 0:
                lines = _batch_lines(proc.stdout, plan)
                if lines:
                    return lines
        except Exception:
            pass

    try:
        cmd = (
            [binary, "scoped", "--limit", str(policy.fetch_k), *scope_args, policy.query]
            if scope_args
            else [binary, "hybrid", policy.query, str(policy.fetch_k)]
        )
        proc = subprocess.run(
            cmd,
            capture_output=True,
            text=True,
            timeout=timeout,
        )
    except Exception:
        return []
    if proc.returncode != 0:
        return []
    return [line for line in proc.stdout.splitlines() if line.strip()]


def parse_hit(line: str) -> tuple[str, float, str, str] | None:
    parts = line.split("\t")
    if len(parts) >= 4:
        try:
            score = float(parts[1])
        except ValueError:
            return None
        return parts[0] or "primary", score, normalize_ws(parts[2]), normalize_ws("\t".join(parts[3:]))
    if len(parts) >= 3:
        try:
            score = float(parts[0])
        except ValueError:
            return None
        return "primary", score, normalize_ws(parts[1]), normalize_ws("\t".join(parts[2:]))
    return None


def centered_snippet(text: str, terms: list[str], max_chars: int) -> str:
    text = normalize_ws(text)
    if len(text) <= max_chars:
        return text
    lower = text.lower()
    pos = -1
    for term in terms:
        pos = lower.find(term.lower())
        if pos >= 0:
            break
    if pos < 0:
        return text[: max_chars - 1].rstrip() + "..."
    start = max(0, pos - max_chars // 3)
    end = min(len(text), start + max_chars)
    start = max(0, end - max_chars)
    snippet = text[start:end].strip()
    if start > 0:
        snippet = "..." + snippet
    if end < len(text):
        snippet += "..."
    return snippet


def adjusted_score(score: float, title: str, snippet: str, terms: list[str], policy: Policy, perspective: str = "primary") -> float:
    hay = f"{title} {snippet}".lower()
    overlap = sum(1 for t in terms if t and t.lower() in hay)
    overlap_boost = 0.018 * (overlap / max(1, min(len(terms), 8)))
    type_boost = 0.0
    if policy.name in {"debug", "optimize"} and re.search(r"test|bench|verify|fix|error|perf|latency|recall|token", hay):
        type_boost += 0.006
    if "decision" in hay or "verified" in hay or "preference" in hay:
        type_boost += 0.004
    if perspective == "decision":
        type_boost += 0.004
    if policy.name == "debug" and perspective in {"fix", "code"}:
        type_boost += 0.004
    if policy.name == "optimize" and perspective in {"bench", "learn"}:
        type_boost += 0.004
    if policy.name == "history" and perspective in {"telepathy", "switch"}:
        type_boost += 0.003
    stale_penalty = 0.004 if "[telepathy]" in hay and policy.name != "history" else 0.0
    return score + overlap_boost + type_boost - stale_penalty


def pack_hits(lines: list[str], policy: Policy) -> tuple[list[Hit], int, int]:
    terms = query_terms(policy.query)
    candidates: list[Hit] = []
    naive_tokens = 0
    for line in lines:
        naive_tokens += estimate_tokens(line)
        parsed = parse_hit(line)
        if not parsed:
            continue
        perspective, score, title, text = parsed
        snippet = centered_snippet(text, terms, policy.snippet_chars)
        adj = adjusted_score(score, title, snippet, terms, policy, perspective)
        overlap = sum(1 for t in terms if t and t.lower() in f"{title} {snippet}".lower())
        if overlap == 0 and policy.name != "history" and adj < policy.min_score + 0.006:
            continue
        if score < policy.min_score and adj < policy.min_score + 0.006:
            continue
        body = f"{title}: {snippet}"
        candidates.append(Hit(score, title, text, snippet, adj, estimate_tokens(body) + 8, perspective))

    candidates.sort(key=lambda h: h.adjusted, reverse=True)
    packed: list[Hit] = []
    used_tokens = 34
    seen_titles: set[str] = set()
    seen_snips: set[str] = set()
    seen_perspectives: set[str] = set()

    def try_add(hit: Hit) -> bool:
        nonlocal used_tokens
        title_key = re.sub(r"[^a-z0-9]+", "", hit.title.lower())[:64]
        snip_key = " ".join(query_terms(hit.snippet, 10))
        if title_key and title_key in seen_titles:
            return False
        if snip_key and snip_key in seen_snips:
            return False
        if used_tokens + hit.tokens > policy.max_tokens:
            return False
        packed.append(hit)
        used_tokens += hit.tokens
        seen_titles.add(title_key)
        seen_snips.add(snip_key)
        seen_perspectives.add(hit.perspective)
        return True

    diversity_goal = min(3, len({h.perspective for h in candidates}), policy.max_items)
    for hit in candidates:
        if len(seen_perspectives) >= diversity_goal:
            break
        if hit.perspective not in seen_perspectives:
            try_add(hit)

    for hit in candidates:
        if len(packed) >= policy.max_items:
            break
        try_add(hit)
    return packed, used_tokens, naive_tokens


def render(policy: Policy, packed: list[Hit], used_tokens: int, naive_tokens: int) -> str:
    if not packed:
        return ""
    saved = max(0, naive_tokens - used_tokens)
    ratio = 0 if naive_tokens <= 0 else round(saved * 100 / naive_tokens)
    lines = [
        f'<synapse_context class="{policy.name}" tokens_est="{used_tokens}" saved_vs_candidates="{ratio}%" perspectives="{",".join(sorted({h.perspective for h in packed}))}">',
    ]
    for hit in packed:
        lines.append(f"- [{hit.perspective} {hit.score:.3f}->{hit.adjusted:.3f}] {hit.title}: {hit.snippet}")
    lines.append("</synapse_context>")
    return "\n".join(lines)


def log_context_event(
    mode: str,
    policy: Policy,
    project: str | None,
    session_id: str | None,
    packed: list[Hit],
    used_tokens: int,
    naive_tokens: int,
    latency_ms: float,
) -> str | None:
    if not learning_enabled() or not packed:
        return None
    ts = int(time())
    perspectives = sorted({hit.perspective for hit in packed})
    hits = [
        {
            "p": hit.perspective,
            "title_hash": stable_hash(hit.title, 12),
            "score": round(hit.score, 6),
            "adjusted": round(hit.adjusted, 6),
            "tokens": hit.tokens,
        }
        for hit in packed[:12]
    ]
    query_hash = stable_hash(policy.query, 20)
    event_id = stable_hash(f"{ts}:{mode}:{policy.name}:{query_hash}:{','.join(perspectives)}:{latency_ms}", 20)
    saved_ratio = 0.0 if naive_tokens <= 0 else max(0.0, (naive_tokens - used_tokens) / naive_tokens)
    try:
        with open_learn_db() as conn:
            conn.execute(
                """
                INSERT OR REPLACE INTO recall_events(
                    id, ts, mode, policy, project, session_id, query_hash, perspectives,
                    hits_json, tokens_est, naive_tokens, saved_ratio, latency_ms
                ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13)
                """,
                (
                    event_id,
                    ts,
                    mode,
                    policy.name,
                    project,
                    session_id,
                    query_hash,
                    json.dumps(perspectives, ensure_ascii=False),
                    json.dumps(hits, ensure_ascii=False),
                    used_tokens,
                    naive_tokens,
                    saved_ratio,
                    latency_ms,
                ),
            )
            for hit in packed:
                conn.execute(
                    """
                    INSERT INTO perspective_bandit(policy, perspective, alpha, beta, uses, rewards, updated_ts)
                    VALUES(?1, ?2, 1.0, 1.0, 1, 0, ?3)
                    ON CONFLICT(policy, perspective) DO UPDATE SET
                        uses = uses + 1,
                        updated_ts = excluded.updated_ts
                    """,
                    (policy.name, hit.perspective, ts),
                )
    except Exception:
        return None
    return event_id


def infer_stop_reward(raw: str) -> float | None:
    lower = raw.lower()
    success = bool(
        re.search(
            r"\b(done|passed|green|verified|fixed|implemented|built|installed|ok|success|"
            r"erledigt|gebaut|verifiziert|funktioniert|gruen|grün|tests? (passed|ok))\b",
            lower,
        )
    )
    failure = bool(
        re.search(
            r"\b(blocked|failed|failing|error|traceback|panic|could not|unable|"
            r"nicht geschafft|blocker|fehlgeschlagen|gescheitert)\b",
            lower,
        )
    )
    if success and not failure:
        return 1.0
    if failure and not success:
        return 0.0
    if success and failure:
        return 0.55
    return None


def apply_reward_to_conn(
    conn: sqlite3.Connection,
    policy: str,
    perspectives: list[str],
    reward: float,
    strength: float = 0.35,
) -> None:
    if not perspectives:
        return
    reward = max(0.0, min(1.0, reward))
    ts = int(time())
    for perspective in sorted(set(perspectives)):
        conn.execute(
            """
            INSERT INTO perspective_bandit(policy, perspective, alpha, beta, uses, rewards, updated_ts)
            VALUES(?1, ?2, ?3, ?4, 1, 1, ?5)
            ON CONFLICT(policy, perspective) DO UPDATE SET
                alpha = alpha + ?3,
                beta = beta + ?4,
                rewards = rewards + 1,
                updated_ts = excluded.updated_ts
            """,
            (policy, perspective, reward * strength, (1.0 - reward) * strength, ts),
        )


def update_perspective_reward(policy: str, perspectives: list[str], reward: float, strength: float = 0.35) -> None:
    if not learning_enabled() or not perspectives:
        return
    try:
        with open_learn_db() as conn:
            apply_reward_to_conn(conn, policy, perspectives, reward, strength)
    except Exception:
        return


def reward_recent(raw: str, project: str | None = None, session_id: str | None = None) -> int:
    reward = infer_stop_reward(raw)
    if reward is None or not learning_enabled():
        return 0
    cutoff = int(time()) - int(os.environ.get("SYNAPSE_RECALL_REWARD_WINDOW_SEC", "43200"))
    try:
        with open_learn_db() as conn:
            clauses = ["ts >= ?1", "reward IS NULL"]
            params: list[object] = [cutoff]
            if session_id:
                clauses.append("session_id = ?{}".format(len(params) + 1))
                params.append(session_id)
            elif project:
                clauses.append("(project = ?{} OR project IS NULL)".format(len(params) + 1))
                params.append(project)
            rows = conn.execute(
                f"""
                SELECT id, policy, perspectives
                FROM recall_events
                WHERE {' AND '.join(clauses)}
                ORDER BY ts DESC
                LIMIT 8
                """,
                params,
            ).fetchall()
            for event_id, policy, perspectives_json in rows:
                try:
                    perspectives = json.loads(perspectives_json)
                except Exception:
                    perspectives = []
                if isinstance(perspectives, list):
                    apply_reward_to_conn(conn, str(policy), [str(p) for p in perspectives], reward)
                conn.execute(
                    "UPDATE recall_events SET reward=?1, reward_ts=?2 WHERE id=?3",
                    (reward, int(time()), event_id),
                )
    except Exception:
        return 0
    return len(rows)


def learn_stats() -> str:
    try:
        with open_learn_db() as conn:
            rows = conn.execute(
                """
                SELECT policy, perspective, alpha, beta, uses, rewards,
                       alpha / (alpha + beta) AS mean
                FROM perspective_bandit
                ORDER BY policy, mean DESC, uses DESC
                """
            ).fetchall()
            events = conn.execute("SELECT COUNT(*), COALESCE(AVG(latency_ms), 0), COALESCE(AVG(saved_ratio), 0) FROM recall_events").fetchone()
    except Exception as exc:
        return json.dumps({"error": str(exc)}, ensure_ascii=False)
    return json.dumps(
        {
            "events": {
                "count": int(events[0] or 0),
                "avg_latency_ms": round(float(events[1] or 0.0), 3),
                "avg_saved_ratio": round(float(events[2] or 0.0), 3),
            },
            "bandit": [
                {
                    "policy": policy,
                    "perspective": perspective,
                    "alpha": round(float(alpha), 3),
                    "beta": round(float(beta), 3),
                    "uses": int(uses),
                    "rewards": int(rewards),
                    "mean": round(float(mean), 3),
                }
                for policy, perspective, alpha, beta, uses, rewards, mean in rows
            ],
        },
        ensure_ascii=False,
        indent=2,
    )


FRESH_KEYWORDS = re.compile(
    r"\b(latest|current|version|versions|upgrade|update|install|package|dependency|"
    r"api|docs|documentation|framework|library|npm|cargo|crate|pip|pypi|pyproject|"
    r"package\.json|cargo\.toml|requirements|context7|fresh|neueste|aktuell|"
    r"versionen|paket|abh.ngigkeit|abhängigkeit|doku|schnittstelle)\b",
    re.I,
)

EDGE_KEYWORDS = re.compile(
    r"\b(agent memory|memory system|persistent memory|recall|context7|context engineering|"
    r"fresh docs|latest docs|omega|omega-memory|omega memory|cortext|cortex|ctx|leanctx|"
    r"lean-ctx|context runtime|docfork|gitmcp|deepwiki|mem0|zep|graphiti|letta|cognee|"
    r"hindsight|supermemory|signet|mcp memory|version slippage|slippage)\b",
    re.I,
)

EDGE_STACK_CANDIDATES = [
    {
        "name": "Synapse",
        "role": "primary local recall + freshness brain",
        "edge": "single-file local memory, fast hybrid recall, Claude/Codex hooks, version-pinned fresh context",
        "action": "keep as default substrate; use other tools as evidence/import/adapters",
    },
    {
        "name": "OMEGA Memory",
        "role": "external local-first MCP memory reference",
        "edge": "SQLite + sqlite-vec + ONNX embeddings, rich MCP tool surface, contradiction/forgetting patterns",
        "action": "installed isolated at .tools/omega-memory; mine patterns, do not replace Synapse hot path",
    },
    {
        "name": "LeanCTX",
        "role": "context compression/runtime candidate",
        "edge": "AST/file/shell compression, token-governed read modes, broad coding-tool compatibility",
        "action": "benchmark as optional context compressor before adding as dependency",
    },
    {
        "name": "Context/Docfork/GitMCP",
        "role": "fresh documentation retrieval layer",
        "edge": "offline/local docs, cabinets/project isolation, zero-setup public GitHub docs",
        "action": "Synapse fresh-context owns local/resolved versions; use these as remote fallback/index sources",
    },
    {
        "name": "Zep/Graphiti",
        "role": "temporal graph reference",
        "edge": "time-aware entity/fact evolution and contradiction handling",
        "action": "copy temporal-validity patterns into Synapse graph, avoid Neo4j dependency for local default",
    },
    {
        "name": "Mem0/Letta/Cognee/Hindsight/Signet",
        "role": "benchmark and API-shape competitors",
        "edge": "managed distribution, agent OS memory blocks, KG/RAG pipelines, multi-strategy retrieval",
        "action": "track as fair-bench targets; integrate compatible API adapters only after recall gates pass",
    },
]


BROKEN_NAMES = {
    "chromadb": "avoid: broken/reliability issues locally; prefer Qdrant or LanceDB",
    "chroma": "avoid for new local agent memory unless explicitly testing Chroma; prefer Qdrant/LanceDB/Synapse",
    "weaviate": "avoid: heavy single-node footprint here",
    "langchain": "avoid: deprecated local preference; prefer direct SDKs, DSPy/LangGraph only when needed",
    "selenium": "avoid: legacy browser automation; prefer Playwright/Patchwright/nodriver",
    "smollm2": "avoid for instruction-critical extraction in this environment",
}


def freshness_needed(prompt: str, mode: str) -> bool:
    if os.environ.get("SYNAPSE_FRESH_CONTEXT") == "0":
        return False
    if mode == "session":
        return os.environ.get("SYNAPSE_FRESH_ON_SESSION", "1") == "1"
    return bool(FRESH_KEYWORDS.search(prompt))


def edge_stack_needed(prompt: str, mode: str) -> bool:
    if os.environ.get("SYNAPSE_EDGE_CONTEXT") == "0":
        return False
    if mode == "session":
        return os.environ.get("SYNAPSE_EDGE_ON_SESSION", "1") == "1"
    return bool(EDGE_KEYWORDS.search(prompt))


def edge_stack_context_block(prompt: str, mode: str) -> str:
    if not edge_stack_needed(prompt, mode):
        return ""
    max_items = int(os.environ.get("SYNAPSE_EDGE_MAX_ITEMS", "6"))
    lines = [
        '<edge_stack_context class="local_recall_freshness_moat" ttl_sec="86400">',
        "- Rule: Synapse remains the local-first hot path; competitor tools are mined for patterns, adapters, docs, and benchmarks before becoming runtime dependencies.",
        "- Levers: zero-friction local default; compounding freshness+recall evidence loop.",
    ]
    lower = prompt.lower()
    if "omega" in lower:
        lines.append("- local setup: OMEGA Memory is installed in .tools/omega-memory and registered as optional Codex MCP server omega-memory.")
    if "context7" in lower or "slippage" in lower or "latest" in lower or "docs" in lower:
        lines.append("- freshness setup: prefer resolved/local package versions from manifests/lockfiles; use remote doc tools only as fallback evidence.")
    if "cortext" in lower:
        lines.append("- spelling note: treat cortext as both omega-cortex and CTX/context-runtime candidates until the user specifies otherwise.")
    for item in EDGE_STACK_CANDIDATES[:max_items]:
        lines.append(
            f"- {item['name']}: role={item['role']}; edge={item['edge']}; action={item['action']}"
        )
    lines.append("</edge_stack_context>")
    return "\n".join(lines)


def native_fresh_context_block(prompt: str, mode: str, cwd: str | None, project: str | None = None) -> str | None:
    if os.environ.get("SYNAPSE_FRESH_NATIVE", "1") == "0":
        return None
    exe = synx_fresh_bin()
    if not exe:
        return None

    cmd = [exe, "fresh-context", "--mode", mode]
    stdin_payload = None
    if len(prompt) > 4000:
        stdin_payload = json.dumps({"prompt": prompt, "cwd": cwd, "project": project}, ensure_ascii=False)
    else:
        cmd.extend(["--prompt", prompt])
        if cwd:
            cmd.extend(["--cwd", cwd])
        if project:
            cmd.extend(["--project", project])
    if os.environ.get("SYNAPSE_FRESH_NO_REGISTRY") == "1":
        cmd.append("--no-registry")

    try:
        proc = subprocess.run(
            cmd,
            input=stdin_payload,
            stdout=subprocess.PIPE,
            stderr=subprocess.DEVNULL,
            text=True,
            timeout=float(os.environ.get("SYNAPSE_FRESH_NATIVE_TIMEOUT", "2.5")),
            check=False,
        )
    except (OSError, subprocess.SubprocessError, ValueError):
        return None
    if proc.returncode == 0:
        return proc.stdout.strip()
    return None


def fresh_db_path() -> Path:
    explicit = os.environ.get("SYNAPSE_FRESH_CONTEXT_DB")
    if explicit:
        return Path(explicit).expanduser()
    return Path.home() / ".synapse" / "fresh_context.db"


def connect_fresh_db() -> sqlite3.Connection:
    path = fresh_db_path()
    path.parent.mkdir(parents=True, exist_ok=True)
    conn = sqlite3.connect(path, timeout=0.2)
    conn.execute("PRAGMA journal_mode=WAL")
    conn.execute("PRAGMA synchronous=NORMAL")
    conn.execute(
        """
        CREATE TABLE IF NOT EXISTS registry_cache (
            ecosystem TEXT NOT NULL,
            name TEXT NOT NULL,
            latest TEXT,
            source TEXT NOT NULL,
            docs TEXT,
            ts INTEGER NOT NULL,
            PRIMARY KEY(ecosystem, name)
        )
        """
    )
    return conn


@contextmanager
def open_fresh_db():
    conn = connect_fresh_db()
    try:
        yield conn
        conn.commit()
    finally:
        conn.close()


@contextmanager
def open_optional_fresh_db():
    try:
        conn = connect_fresh_db()
    except Exception:
        yield None
        return
    try:
        yield conn
        conn.commit()
    finally:
        conn.close()


def project_root_from_cwd(cwd: str | None) -> Path:
    start = Path(cwd or os.getcwd()).expanduser().resolve()
    if start.is_file():
        start = start.parent
    markers = {"Cargo.toml", "package.json", "pyproject.toml", "requirements.txt", ".git"}
    cur = start
    while True:
        if any((cur / marker).exists() for marker in markers):
            return cur
        if cur.parent == cur:
            return start
        cur = cur.parent


def parse_dep_spec(value: object) -> str:
    if isinstance(value, str):
        return value
    if isinstance(value, dict):
        if "version" in value:
            return str(value["version"])
        if "path" in value:
            return f"path:{value['path']}"
        if "git" in value:
            return f"git:{value['git']}"
    return str(value)


def parse_cargo_manifest(path: Path) -> list[Dep]:
    try:
        data = tomllib.loads(path.read_text(errors="ignore"))
    except Exception:
        return []
    deps: list[Dep] = []
    for section in ("dependencies", "dev-dependencies", "build-dependencies"):
        for name, value in data.get(section, {}).items():
            deps.append(Dep("crates", str(name), parse_dep_spec(value), manifest=str(path)))
    workspace = data.get("workspace", {})
    for name, value in workspace.get("dependencies", {}).items():
        deps.append(Dep("crates", str(name), parse_dep_spec(value), manifest=str(path)))
    return deps


def parse_package_json(path: Path) -> list[Dep]:
    try:
        data = json.loads(path.read_text(errors="ignore"))
    except Exception:
        return []
    deps: list[Dep] = []
    for section in ("dependencies", "devDependencies", "peerDependencies", "optionalDependencies"):
        for name, version in data.get(section, {}).items():
            deps.append(Dep("npm", str(name), str(version), manifest=str(path)))
    return deps


def pep_dep_name(spec: str) -> tuple[str, str] | None:
    spec = spec.strip()
    if not spec or spec.startswith("#") or spec.startswith("-"):
        return None
    m = re.match(r"([A-Za-z0-9_.-]+)\s*(.*)", spec)
    if not m:
        return None
    return m.group(1), m.group(2).strip() or "*"


def parse_pyproject(path: Path) -> list[Dep]:
    try:
        data = tomllib.loads(path.read_text(errors="ignore"))
    except Exception:
        return []
    deps: list[Dep] = []
    for spec in data.get("project", {}).get("dependencies", []) or []:
        parsed = pep_dep_name(str(spec))
        if parsed:
            deps.append(Dep("pypi", parsed[0], parsed[1], manifest=str(path)))
    optional = data.get("project", {}).get("optional-dependencies", {}) or {}
    for specs in optional.values():
        for spec in specs or []:
            parsed = pep_dep_name(str(spec))
            if parsed:
                deps.append(Dep("pypi", parsed[0], parsed[1], manifest=str(path)))
    poetry = data.get("tool", {}).get("poetry", {}).get("dependencies", {}) or {}
    for name, value in poetry.items():
        if str(name).lower() != "python":
            deps.append(Dep("pypi", str(name), parse_dep_spec(value), manifest=str(path)))
    return deps


def parse_requirements(path: Path) -> list[Dep]:
    deps: list[Dep] = []
    try:
        lines = path.read_text(errors="ignore").splitlines()
    except Exception:
        return deps
    for line in lines:
        parsed = pep_dep_name(line)
        if parsed:
            deps.append(Dep("pypi", parsed[0], parsed[1], manifest=str(path)))
    return deps


def parse_cargo_lock(path: Path) -> dict[str, str]:
    return parse_package_toml_lock(path)


def parse_package_toml_lock(path: Path) -> dict[str, str]:
    out: dict[str, str] = {}
    name: str | None = None
    version: str | None = None

    def flush() -> None:
        if name and version:
            out[name] = version

    try:
        with path.open("r", encoding="utf-8", errors="ignore") as fh:
            for raw in fh:
                line = raw.strip()
                if line == "[[package]]":
                    flush()
                    name = None
                    version = None
                elif line.startswith("name = "):
                    name = line.split("=", 1)[1].strip().strip('"')
                elif line.startswith("version = "):
                    version = line.split("=", 1)[1].strip().strip('"')
        flush()
    except Exception:
        return {}
    return out


def parse_package_lock(path: Path) -> dict[str, str]:
    try:
        data = json.loads(path.read_text(errors="ignore"))
    except Exception:
        return {}
    out = {}
    for loc, meta in (data.get("packages") or {}).items():
        if not loc.startswith("node_modules/") or not isinstance(meta, dict):
            continue
        name = loc.removeprefix("node_modules/")
        version = meta.get("version")
        if name and version:
            out[name] = str(version)
    for name, meta in (data.get("dependencies") or {}).items():
        if isinstance(meta, dict) and meta.get("version"):
            out.setdefault(str(name), str(meta["version"]))
    return out


def parse_uv_lock(path: Path) -> dict[str, str]:
    return parse_package_toml_lock(path)


def nearest_manifests(root: Path) -> list[Path]:
    names = {"Cargo.toml", "package.json", "pyproject.toml", "requirements.txt"}
    found: list[Path] = []
    for name in sorted(names):
        p = root / name
        if p.exists():
            found.append(p)
    max_files = int(os.environ.get("SYNAPSE_FRESH_MAX_MANIFESTS", "8"))
    if found and os.environ.get("SYNAPSE_FRESH_SCAN_SUBPROJECTS") != "1":
        return found[:max_files]
    if len(found) >= max_files:
        return found[:max_files]
    max_depth = int(os.environ.get("SYNAPSE_FRESH_MAX_DEPTH", "3"))
    skip = {"node_modules", "target", ".git", "__pycache__", ".venv", "dist", "build", ".next"}
    seen = set(found)
    for current, dirs, files in os.walk(root):
        cur = Path(current)
        try:
            depth = len(cur.relative_to(root).parts)
        except ValueError:
            depth = 0
        dirs[:] = [d for d in dirs if d not in skip and depth < max_depth]
        for name in sorted(names.intersection(files)):
            path = cur / name
            if path not in seen:
                found.append(path)
                seen.add(path)
                if len(found) >= max_files:
                    return found[:max_files]
    return found


def collect_project_deps(cwd: str | None, prompt: str) -> list[Dep]:
    root = project_root_from_cwd(cwd)
    deps: list[Dep] = []
    for manifest in nearest_manifests(root):
        if manifest.name == "Cargo.toml":
            deps.extend(parse_cargo_manifest(manifest))
        elif manifest.name == "package.json":
            deps.extend(parse_package_json(manifest))
        elif manifest.name == "pyproject.toml":
            deps.extend(parse_pyproject(manifest))
        elif manifest.name.startswith("requirements"):
            deps.extend(parse_requirements(manifest))
    resolved: dict[tuple[str, str], str] = {}
    for lock_name, parser, ecosystem in (
        ("Cargo.lock", parse_cargo_lock, "crates"),
        ("package-lock.json", parse_package_lock, "npm"),
        ("uv.lock", parse_uv_lock, "pypi"),
    ):
        p = root / lock_name
        if p.exists():
            for name, version in parser(p).items():
                resolved[(ecosystem, name.lower())] = version
    out = []
    seen: set[tuple[str, str]] = set()
    terms = set(query_terms(prompt, 30))
    for dep in deps:
        key = (dep.ecosystem, dep.name.lower())
        if key in seen:
            continue
        seen.add(key)
        resolved_version = resolved.get(key)
        out.append(Dep(dep.ecosystem, dep.name, dep.declared, resolved_version, dep.manifest))
    if terms:
        selected = [d for d in out if d.name.lower() in terms or any(part.lower() in terms for part in re.split(r"[-_/@.]+", d.name) if part)]
        if selected:
            return selected[: int(os.environ.get("SYNAPSE_FRESH_MAX_DEPS", "8"))]
    return out[: int(os.environ.get("SYNAPSE_FRESH_MAX_DEPS", "8"))]


def exact_version(value: str | None) -> str | None:
    if not value:
        return None
    value = value.strip()
    return value if re.fullmatch(r"[0-9]+(?:\.[0-9A-Za-z][0-9A-Za-z.-]*)*", value) else None


def docs_url(ecosystem: str, name: str, version: str | None = None) -> str | None:
    pinned = exact_version(version)
    if ecosystem == "crates":
        return f"https://docs.rs/{name}/{pinned}/" if pinned else f"https://docs.rs/{name}/latest/"
    if ecosystem == "npm":
        pkg = urllib.parse.quote(name, safe="@/")
        return f"https://www.npmjs.com/package/{pkg}/v/{pinned}" if pinned else f"https://www.npmjs.com/package/{pkg}"
    if ecosystem == "pypi":
        pkg = urllib.parse.quote(name)
        return f"https://pypi.org/project/{pkg}/{pinned}/" if pinned else f"https://pypi.org/project/{pkg}/"
    return None


def registry_url(ecosystem: str, name: str) -> str:
    if ecosystem == "crates":
        return f"https://crates.io/api/v1/crates/{urllib.parse.quote(name)}"
    if ecosystem == "npm":
        return f"https://registry.npmjs.org/{urllib.parse.quote(name, safe='')}/latest"
    if ecosystem == "pypi":
        return f"https://pypi.org/pypi/{urllib.parse.quote(name)}/json"
    raise ValueError(ecosystem)


def fetch_registry_latest(ecosystem: str, name: str, conn: sqlite3.Connection | None = None) -> RegistryInfo:
    ttl = int(os.environ.get("SYNAPSE_FRESH_TTL_SEC", "21600"))
    negative_ttl = int(os.environ.get("SYNAPSE_FRESH_NEG_TTL_SEC", "60"))
    now = int(time())
    docs = docs_url(ecosystem, name)
    try:
        if conn is not None:
            row = conn.execute(
                "SELECT latest, source, docs, ts FROM registry_cache WHERE ecosystem=?1 AND name=?2",
                (ecosystem, name.lower()),
            ).fetchone()
        else:
            with open_fresh_db() as cache:
                row = cache.execute(
                    "SELECT latest, source, docs, ts FROM registry_cache WHERE ecosystem=?1 AND name=?2",
                    (ecosystem, name.lower()),
                ).fetchone()
        if row and now - int(row[3]) < (ttl if row[0] else negative_ttl):
            return RegistryInfo(row[0], row[1], row[2] or docs, cached=True)
    except Exception:
        pass

    url = registry_url(ecosystem, name)
    latest = None
    source = url
    timeout = float(os.environ.get("SYNAPSE_FRESH_TIMEOUT", "0.75"))
    try:
        req = urllib.request.Request(url, headers={"User-Agent": "synapse-fresh-context/1.0"})
        with urllib.request.urlopen(req, timeout=timeout) as resp:
            data = json.loads(resp.read().decode("utf-8", "ignore"))
        if ecosystem == "crates":
            crate = data.get("crate", {})
            latest = crate.get("max_stable_version") or crate.get("newest_version") or crate.get("max_version")
        elif ecosystem in {"npm", "pypi"}:
            latest = data.get("version") if ecosystem == "npm" else data.get("info", {}).get("version")
    except (OSError, urllib.error.URLError, ValueError, json.JSONDecodeError):
        latest = None

    try:
        if conn is not None:
            conn.execute(
                """
                INSERT OR REPLACE INTO registry_cache(ecosystem, name, latest, source, docs, ts)
                VALUES(?1, ?2, ?3, ?4, ?5, ?6)
                """,
                (ecosystem, name.lower(), latest, source, docs, now),
            )
        else:
            with open_fresh_db() as cache:
                cache.execute(
                    """
                    INSERT OR REPLACE INTO registry_cache(ecosystem, name, latest, source, docs, ts)
                    VALUES(?1, ?2, ?3, ?4, ?5, ?6)
                    """,
                    (ecosystem, name.lower(), latest, source, docs, now),
                )
    except Exception:
        pass
    return RegistryInfo(latest, source, docs, cached=False)


def version_slip(dep: Dep, info: RegistryInfo) -> str:
    local = dep.resolved or dep.declared
    if not info.latest:
        return "latest_unknown"
    pinned = exact_version(local)
    if pinned and pinned != info.latest:
        return "pinned_differs"
    if dep.resolved and dep.resolved != info.latest:
        return "resolved_differs"
    return "ok_or_range"


def fresh_context_block(prompt: str, mode: str, cwd: str | None, project: str | None = None) -> str:
    if not freshness_needed(prompt, mode):
        return ""
    native = native_fresh_context_block(prompt, mode, cwd, project)
    if native is not None:
        return native
    deps = collect_project_deps(cwd, prompt)
    if not deps:
        return ""
    max_registry = int(os.environ.get("SYNAPSE_FRESH_MAX_REGISTRY", "3" if mode == "session" else "5"))
    lines = [
        f'<fresh_context class="version_guard" project="{project or Path(cwd or os.getcwd()).name}" ttl_sec="{os.environ.get("SYNAPSE_FRESH_TTL_SEC", "21600")}">',
        "- Rule: code against the resolved/local version unless explicitly upgrading; if registry latest differs, avoid using APIs that only exist in the latest release.",
    ]
    broken_hits = [name for name in BROKEN_NAMES if re.search(rf"(^|[^A-Za-z0-9_.-]){re.escape(name)}([^A-Za-z0-9_.-]|$)", prompt, re.I)]
    for name in broken_hits[:4]:
        lines.append(f"- broken/avoid {name}: {BROKEN_NAMES[name]}")
    cache_ctx = open_optional_fresh_db() if max_registry > 0 else nullcontext(None)
    with cache_ctx as conn:
        for idx, dep in enumerate(deps):
            info = fetch_registry_latest(dep.ecosystem, dep.name, conn) if idx < max_registry else RegistryInfo(None, "not_checked", docs_url(dep.ecosystem, dep.name), False)
            local = dep.resolved or dep.declared
            slip = version_slip(dep, info)
            cache = " cached" if info.cached else ""
            latest = info.latest or "unknown"
            docs = docs_url(dep.ecosystem, dep.name, local) or info.docs
            docs = f" docs={docs}" if docs else ""
            lines.append(
                f"- {dep.ecosystem}:{dep.name} local={local} declared={dep.declared} latest={latest} status={slip}{cache}{docs}"
            )
    lines.append("</fresh_context>")
    return "\n".join(lines)


def build_context_details(raw: str, mode: str, project: str | None = None) -> ContextBuild | None:
    t0 = perf_counter_ns()
    prompt, sid, cwd = parse_hook_input(raw)
    if not project and cwd:
        project = Path(cwd).name
    policy = classify_prompt(prompt, mode, project)
    fresh = fresh_context_block(prompt, mode, cwd, project)
    edge = edge_stack_context_block(prompt, mode)
    if policy is None:
        text = "\n".join(block for block in (fresh, edge) if block)
        return (
            ContextBuild(text, Policy("fresh", prompt[:120], 0, estimate_tokens(text), 0.0, 0, 0), [], estimate_tokens(text), estimate_tokens(text), 0.0)
            if text
            else None
        )
    lines = fetch_hits(policy, project=project)
    packed: list[Hit] = []
    used_tokens = 0
    naive_tokens = 0
    text = ""
    if lines:
        packed, used_tokens, naive_tokens = pack_hits(lines, policy)
        text = render(policy, packed, used_tokens, naive_tokens)
    if fresh:
        text = f"{fresh}\n{text}" if text else fresh
        used_tokens += estimate_tokens(fresh)
        naive_tokens += estimate_tokens(fresh)
    if edge:
        text = f"{edge}\n{text}" if text else edge
        used_tokens += estimate_tokens(edge)
        naive_tokens += estimate_tokens(edge)
    if not text:
        return None
    latency_ms = (perf_counter_ns() - t0) / 1_000_000
    event_id = log_context_event(mode, policy, project, sid, packed, used_tokens, naive_tokens, latency_ms) if packed else None
    return ContextBuild(text, policy, packed, used_tokens, naive_tokens, latency_ms, event_id)


def build_context(raw: str, mode: str, project: str | None = None) -> str:
    built = build_context_details(raw, mode, project)
    return "" if built is None else built.text


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("--mode", choices=["prompt", "session"], default="prompt")
    parser.add_argument("--project")
    parser.add_argument("--reward-stop", action="store_true")
    parser.add_argument("--learn-stats", action="store_true")
    parser.add_argument("--fresh-context", action="store_true")
    parser.add_argument("--edge-context", action="store_true")
    args = parser.parse_args()
    raw = sys.stdin.read()
    if args.learn_stats:
        print(learn_stats())
        return 0
    if args.fresh_context:
        prompt, _sid, cwd = parse_hook_input(raw)
        project = args.project or (Path(cwd).name if cwd else Path(os.getcwd()).name)
        out = fresh_context_block(prompt or raw, args.mode, cwd, project)
        if out:
            print(out)
        return 0
    if args.edge_context:
        prompt, _sid, _cwd = parse_hook_input(raw)
        out = edge_stack_context_block(prompt or raw, args.mode)
        if out:
            print(out)
        return 0
    if args.reward_stop:
        prompt, sid, cwd = parse_hook_input(raw)
        project = args.project or (Path(cwd).name if cwd else Path(os.getcwd()).name)
        count = reward_recent(prompt or raw, project=project, session_id=sid)
        if os.environ.get("SYNAPSE_CONTEXT_DEBUG") == "1":
            print(json.dumps({"rewarded": count}, ensure_ascii=False))
        return 0
    out = build_context(raw, args.mode, args.project)
    if out:
        print(out)
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
