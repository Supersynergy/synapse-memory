"""Synapse unix-socket client. Length-prefixed msgpack protocol."""
import html
import json
import os
import re
import socket
import struct
import time
from typing import Any, Optional, Union, List, Dict

import msgpack

DEFAULT_SOCK = "/tmp/synapse.sock"


AGENT_RECALL_LEXICON = {
    "context": {"compact", "hydrate", "index", "observations", "progressive", "disclosure"},
    "tokens": {"compact", "hydrate", "index", "observations", "progressive", "disclosure"},
    "token": {"compact", "hydrate", "index", "observations", "progressive", "disclosure"},
    "save": {"compact", "hydrate", "index", "observations", "progressive", "disclosure"},
    "slippage": {"freshness", "source", "source_uri", "package", "versions", "local"},
    "version": {"freshness", "source", "source_uri", "package", "versions", "local"},
    "versions": {"freshness", "source", "source_uri", "package", "versions", "local"},
    "feedback": {"accepted", "rejected", "edits", "tests", "rerank", "reranking"},
    "rerank": {"accepted", "rejected", "edits", "tests", "feedback", "reranking"},
    "fallback": {"lexical", "scoped", "embedding", "embeddings", "degrade"},
    "embeddings": {"lexical", "scoped", "embedding", "fallback", "degrade"},
    "deployment": {"single", "file", "unix", "socket", "local"},
    "local": {"single", "file", "unix", "socket", "deployment"},
    "graph": {"temporal", "enrichment", "behind", "hot", "path"},
    "temporal": {"graph", "enrichment", "behind", "hot", "path"},
}


class SynapseError(RuntimeError):
    pass


class Client:
    """Synapse daemon client via unix socket.

    Args:
        sock_path: Path to unix socket (default: /tmp/synapse.sock)
        timeout: Per-call timeout seconds (default: 30, extended for batch)

    Measured latencies (M4 Max, BGE-small ONNX, 2026-04-20):
        ping   p50 58µs
        hybrid p50 8.2ms
        put    p50 335ms (fresh embed) | <1ms (cache hit)
    """

    def __init__(self, sock_path: Optional[str] = None, timeout: float = 30.0,
                 api_key: Optional[str] = None):
        self.sock_path = sock_path or os.environ.get("SYNAPSE_SOCK", DEFAULT_SOCK)
        self.timeout = timeout
        self.api_key = api_key if api_key is not None else os.environ.get("SYNAPSE_API_KEY")

    def _call(self, req: Dict[str, Any], timeout: Optional[float] = None) -> Any:
        s = socket.socket(socket.AF_UNIX, socket.SOCK_STREAM)
        s.settimeout(timeout or self.timeout)
        try:
            s.connect(self.sock_path)
        except (ConnectionRefusedError, FileNotFoundError) as e:
            raise SynapseError(f"daemon not running at {self.sock_path}: {e}") from e
        try:
            if self.api_key:
                auth = self._roundtrip(s, {"op": "Auth", "args": {"token": self.api_key}})
                if isinstance(auth, dict) and "Err" in auth:
                    raise SynapseError(auth["Err"])
            resp = self._roundtrip(s, req)
        finally:
            s.close()
        if isinstance(resp, dict) and "Err" in resp:
            raise SynapseError(resp["Err"])
        return resp

    def _roundtrip(self, s: socket.socket, req: Dict[str, Any]) -> Any:
        body = msgpack.packb(req)
        s.sendall(struct.pack("<I", len(body)) + body)
        hdr = self._recv_n(s, 4)
        n = struct.unpack("<I", hdr)[0]
        buf = self._recv_n(s, n)
        return msgpack.unpackb(buf, raw=False)

    @staticmethod
    def _recv_n(s: socket.socket, n: int) -> bytes:
        buf = b""
        while len(buf) < n:
            chunk = s.recv(n - len(buf))
            if not chunk:
                raise SynapseError("connection closed")
            buf += chunk
        return buf

    # --- API ---

    @staticmethod
    def _mode(mode: str) -> str:
        mode_map = {"hybrid": "Hybrid", "lex": "Lex", "vec": "Vec"}
        return mode_map.get(mode.lower(), "Hybrid")

    def ping(self) -> bool:
        return self._call({"op": "Ping"}) == "Pong"

    def stats(self) -> Dict[str, int]:
        r = self._call({"op": "Stats"})
        return r.get("Stats", r)

    def put(self, text: str, title: Optional[str] = None,
            uri: Optional[str] = None, meta: Optional[Dict] = None,
            embed: bool = True) -> int:
        req = {"op": "Put", "args": {"title": title, "uri": uri,
                                      "text": text, "meta": meta, "embed": embed}}
        r = self._call(req)
        return r.get("Id", r)

    def put_batch(self, items: List[Dict[str, Any]],
                  embed: bool = True, timeout: float = 600.0) -> List[int]:
        """Bulk insert. Each item: {text, title?, uri?, meta?}.

        10-100× faster than per-item put for unique content.
        Hits embedding cache for duplicates (~707k docs/s cache-hit).
        """
        batch = [{"title": it.get("title"), "uri": it.get("uri"),
                  "text": it["text"], "meta": it.get("meta"), "embed": embed}
                 for it in items]
        r = self._call({"op": "PutBatch", "args": batch}, timeout=timeout)
        return r.get("Ids", r)

    def search(self, query: str, mode: str = "hybrid", limit: int = 10,
               embed_query: bool = True, include_meta: bool = False) -> List[Dict[str, Any]]:
        """Search memory.

        Args:
            mode: "hybrid" (BM25+vec RRF) | "lex" (FTS5) | "vec" (kNN)
            include_meta: hydrate hit.meta from docs via the daemon SQL op.
        """
        r = self._call({"op": "Search",
                        "args": {"mode": self._mode(mode), "q": query, "limit": int(limit),
                                 "embed_query": embed_query}})
        hits = r.get("Hits", r) or []
        return self._hydrate_meta(hits) if include_meta else hits

    def search_scoped(self, query: str, scope_value: str, scope_key: str = "scope",
                      mode: str = "hybrid", limit: int = 10,
                      embed_query: bool = True,
                      candidate_limit: Optional[int] = None,
                      include_meta: bool = False) -> List[Dict[str, Any]]:
        """Daemon-native scope-first search.

        This is the public hot path for agent/project memory: the daemon filters
        by docs.meta before fallback ranking, which prevents unrelated global
        memories from hiding the local working set.
        """
        args: Dict[str, Any] = {
            "mode": self._mode(mode),
            "q": query,
            "limit": int(limit),
            "embed_query": embed_query,
            "scope_key": scope_key,
            "scope_value": scope_value,
        }
        if candidate_limit is not None:
            args["candidate_limit"] = int(candidate_limit)
        r = self._call({"op": "SearchScoped", "args": args})
        hits = r.get("Hits", r) or []
        return self._hydrate_meta(hits) if include_meta else hits

    def batch_search(self, queries: List[Union[str, Dict[str, Any]]],
                     mode: str = "hybrid", limit: int = 10,
                     embed_query: bool = True,
                     include_meta: bool = False) -> List[List[Dict[str, Any]]]:
        """Run multiple searches in one daemon/socket roundtrip.

        Each query may be a plain string or a dict with q/query, mode, limit,
        and embed_query. The daemon executes the batch sequentially, but avoids
        N client forks/socket handshakes in hooks and evaluation loops.
        """
        items = []
        for item in queries:
            if isinstance(item, str):
                q = item
                item_mode = mode
                item_limit = limit
                item_embed = embed_query
            else:
                q = str(item.get("q", item.get("query", "")))
                item_mode = str(item.get("mode", mode))
                item_limit = int(item.get("limit", limit))
                item_embed = bool(item.get("embed_query", embed_query))
            items.append({
                "mode": self._mode(item_mode),
                "q": q,
                "limit": item_limit,
                "embed_query": item_embed,
            })
        r = self._call({"op": "BatchSearch", "args": {"queries": items}})
        batches = r.get("BatchHits", r) or []
        if include_meta:
            return [self._hydrate_meta(batch) for batch in batches]
        return batches

    def sql(self, query: str, params: Optional[List[Any]] = None) -> List[Dict[str, Any]]:
        """Run a read-only daemon SQL query and return rows as dictionaries."""
        r = self._call({"op": "Sql", "args": {"query": query, "params": params or []}})
        payload = r.get("Rows", r)
        cols = payload.get("cols", []) if isinstance(payload, dict) else []
        rows = payload.get("rows", []) if isinstance(payload, dict) else []
        return [dict(zip(cols, row)) for row in rows]

    @staticmethod
    def _parse_meta(value: Any) -> Optional[Dict[str, Any]]:
        if isinstance(value, dict):
            return value
        if isinstance(value, str) and value:
            try:
                parsed = json.loads(value)
            except json.JSONDecodeError:
                return None
            return parsed if isinstance(parsed, dict) else None
        return None

    def _hydrate_meta(self, hits: List[Dict[str, Any]]) -> List[Dict[str, Any]]:
        """Attach docs.meta to hits when the daemon search response omits it."""
        ids = []
        out = []
        for hit in hits:
            h = dict(hit)
            out.append(h)
            if not h.get("meta") and h.get("id") is not None:
                try:
                    ids.append(int(h["id"]))
                except (TypeError, ValueError):
                    pass
        ids = list(dict.fromkeys(ids))
        if not ids:
            return out
        placeholders = ",".join("?" for _ in ids)
        try:
            rows = self.sql(f"SELECT id, meta FROM docs WHERE id IN ({placeholders})", ids)
        except Exception:
            return out
        meta_by_id = {
            int(row["id"]): self._parse_meta(row.get("meta"))
            for row in rows
            if row.get("id") is not None
        }
        for hit in out:
            if hit.get("meta") or hit.get("id") is None:
                continue
            try:
                meta = meta_by_id.get(int(hit["id"]))
            except (TypeError, ValueError):
                meta = None
            if meta is not None:
                hit["meta"] = meta
        return out

    @staticmethod
    def _hit_blob(hit: Dict[str, Any]) -> str:
        return "\n".join(str(hit.get(key, "")) for key in (
            "text", "content", "memory", "title", "id", "uri", "answer", "snippet"
        ))

    @staticmethod
    def _query_terms(query: str) -> set[str]:
        terms = {
            term
            for term in re.findall(r"[a-z0-9]+", query.lower().replace("-", " "))
            if len(term) > 2 and not term.isdigit()
        }
        expanded = set(terms)
        for term in terms:
            expanded.update(AGENT_RECALL_LEXICON.get(term, ()))
        return expanded

    def _rank_hits(self, query: str, hits: List[Dict[str, Any]], limit: int) -> List[Dict[str, Any]]:
        terms = self._query_terms(query)
        unique = []
        seen: set[str] = set()
        for hit in hits:
            key = str(hit.get("id", self._hit_blob(hit)))
            if key in seen:
                continue
            seen.add(key)
            unique.append(hit)

        def score(hit: Dict[str, Any]) -> tuple[int, int, float]:
            blob = self._hit_blob(hit).lower()
            exact = 1 if query.lower() in blob else 0
            overlap = sum(1 for term in terms if term in blob)
            return (exact, overlap, float(hit.get("score") or 0.0))

        return sorted(unique, key=score, reverse=True)[:limit]

    def _scope_candidates(self, scope: str, limit: int) -> List[Dict[str, Any]]:
        """Fetch scoped docs directly; this is the fastest path for small banks."""
        select = (
            "SELECT id, uri, title, substr(text,1,4000) AS text, meta "
            "FROM docs "
            "WHERE meta IS NOT NULL AND json_valid(meta) "
            "AND json_extract(meta, '$.scope') = ? "
            "ORDER BY id DESC LIMIT ?"
        )
        try:
            rows = self.sql(select, [scope, int(limit)])
        except Exception:
            like = '%"scope":"' + scope.replace('"', '""') + '"%'
            rows = self.sql(
                "SELECT id, uri, title, substr(text,1,4000) AS text, meta "
                "FROM docs WHERE meta LIKE ? ORDER BY id DESC LIMIT ?",
                [like, int(limit)],
            )
        candidates = []
        for row in rows:
            hit = dict(row)
            hit["meta"] = self._parse_meta(hit.get("meta"))
            if (hit.get("meta") or {}).get("scope") == scope:
                hit.setdefault("score", 0.0)
                candidates.append(hit)
        return candidates

    def _scope_query_candidates(self, query: str, scope: str, limit: int) -> List[Dict[str, Any]]:
        """Fetch scoped docs matching query terms across the whole scope.

        This prevents a large active scope from hiding older but exact memories
        behind the newest `fetch_k` rows.
        """
        terms = list(self._query_terms(query))[:16]
        if not terms:
            return []
        filters = []
        params: List[Any] = [scope]
        for term in terms:
            filters.append("(lower(coalesce(title,'')) LIKE ? OR lower(coalesce(text,'')) LIKE ?)")
            needle = f"%{term.lower()}%"
            params.extend([needle, needle])
        params.append(int(limit))
        select = (
            "SELECT id, uri, title, substr(text,1,4000) AS text, meta "
            "FROM docs "
            "WHERE meta IS NOT NULL AND json_valid(meta) "
            "AND json_extract(meta, '$.scope') = ? "
            f"AND ({' OR '.join(filters)}) "
            "ORDER BY id DESC LIMIT ?"
        )
        rows = self.sql(select, params)
        candidates = []
        for row in rows:
            hit = dict(row)
            hit["meta"] = self._parse_meta(hit.get("meta"))
            if (hit.get("meta") or {}).get("scope") == scope:
                hit.setdefault("score", 0.0)
                candidates.append(hit)
        return candidates

    def search_scoped_fusion(self, query: str, scope: str, limit: int = 5,
                             fetch_k: Optional[int] = None,
                             modes: tuple[str, ...] = ("lex", "hybrid"),
                             embed_query: bool = True) -> List[Dict[str, Any]]:
        """High-precision recall for scoped agent memory.

        This keeps the daemon hot path unchanged while composing four proven
        primitives: indexed scope-first candidate fetch, one-socket BatchSearch
        fallback, docs.meta scope filtering, and a tiny query-term rerank. It is
        the SDK path Bank.recall uses.
        """
        k = max(int(fetch_k or limit * 10), int(limit))
        try:
            native = self.search_scoped(
                query,
                scope,
                mode="hybrid" if "hybrid" in modes else modes[0],
                limit=limit,
                candidate_limit=k,
                embed_query=embed_query,
                include_meta=True,
            )
            if len(native) >= limit:
                return native
        except Exception:
            native = []
        scoped_query = []
        try:
            scoped_query = self._scope_query_candidates(query, scope, max(k, limit))
        except Exception:
            scoped_query = []
        try:
            scoped = self._scope_candidates(scope, max(k, limit))
        except Exception:
            scoped = []
        if scoped_query:
            ranked = self._rank_hits(query, scoped_query + scoped, limit)
            if len(ranked) >= limit:
                return ranked
        batch = [
            {"q": query, "mode": mode, "limit": k, "embed_query": embed_query}
            for mode in modes
        ]
        try:
            batches = self.batch_search(batch, include_meta=True)
        except Exception:
            batches = [
                self.search(query, mode=mode, limit=k, embed_query=embed_query, include_meta=True)
                for mode in modes
            ]
        seen: set[str] = set()
        candidates: List[Dict[str, Any]] = []
        for hits in batches:
            for hit in hits:
                key = str(hit.get("id", self._hit_blob(hit)))
                if key in seen:
                    continue
                seen.add(key)
                if (hit.get("meta") or {}).get("scope") == scope:
                    candidates.append(hit)
        return self._rank_hits(query, native + scoped_query + scoped + candidates, limit)

    def timeline(self, limit: int = 50, offset: int = 0) -> List[Dict]:
        r = self._call({"op": "Timeline", "args": {"limit": limit, "offset": offset}})
        return r.get("Docs", r) or []

    def get_docs(self, ids: List[Union[int, str]],
                 max_chars: Optional[int] = None) -> List[Dict[str, Any]]:
        """Fetch full docs by id, preserving caller order.

        This is the second phase of progressive disclosure: search returns a
        compact index, then agents hydrate only the observations they actually
        need for the current context window.
        """
        wanted: List[int] = []
        for value in ids:
            try:
                doc_id = int(value)
            except (TypeError, ValueError):
                continue
            if doc_id not in wanted:
                wanted.append(doc_id)
        if not wanted:
            return []

        placeholders = ",".join("?" for _ in wanted)
        rows = self.sql(
            "SELECT id, uri, title, text, meta, ts FROM docs "
            f"WHERE id IN ({placeholders})",
            wanted,
        )
        by_id: Dict[int, Dict[str, Any]] = {}
        for row in rows:
            try:
                doc_id = int(row["id"])
            except (KeyError, TypeError, ValueError):
                continue
            doc = dict(row)
            doc["meta"] = self._parse_meta(doc.get("meta"))
            text = str(doc.get("text") or "")
            doc["token_estimate"] = estimate_tokens(text)
            if max_chars is not None and len(text) > max_chars:
                doc["text"] = text[:max_chars].rstrip() + "\n[truncated]"
                doc["truncated"] = True
            by_id[doc_id] = doc
        return [by_id[doc_id] for doc_id in wanted if doc_id in by_id]

    def timeline_scoped(self, scope: str, limit: int = 50, offset: int = 0,
                        kind: Optional[str] = None) -> List[Dict[str, Any]]:
        """Return recent docs for one agent/project scope via indexed metadata."""
        params: List[Any] = [scope]
        where = [
            "meta IS NOT NULL",
            "json_valid(meta)",
            "json_extract(meta, '$.scope') = ?",
        ]
        if kind:
            where.append(
                "(json_extract(meta, '$.kind') = ? OR json_extract(meta, '$.type') = ?)"
            )
            params.extend([kind, kind])
        params.extend([int(limit), int(offset)])
        rows = self.sql(
            "SELECT id, uri, title, substr(text,1,4000) AS text, meta, ts "
            "FROM docs WHERE " + " AND ".join(where) + " "
            "ORDER BY ts DESC, id DESC LIMIT ? OFFSET ?",
            params,
        )
        out = []
        for row in rows:
            doc = dict(row)
            doc["meta"] = self._parse_meta(doc.get("meta"))
            doc["token_estimate"] = estimate_tokens(str(doc.get("text") or ""))
            out.append(doc)
        return out

    def snap(self, out: str, level: int = 3) -> None:
        self._call({"op": "Snap", "args": {"out": out, "level": level}})

    def verify(self, doc_id: int, verifying_key: bytes) -> bool:
        if len(verifying_key) != 32:
            raise ValueError("verifying_key must be 32 bytes")
        r = self._call({"op": "Verify", "args": {"id": doc_id,
                                                   "vk": list(verifying_key)}})
        return r == "Ok"

    # --- scoped bank API (Hindsight-style sugar) ---

    def bank(self, bank_id: str) -> "Bank":
        return Bank(self, bank_id)

    def agent_db(self, agent_id: str, project: Optional[str] = None) -> "AgentDB":
        """Public agent database facade with progressive context packing."""
        return AgentDB(self, agent_id=agent_id, project=project)

    def agent(self, agent_id: str, project: Optional[str] = None) -> "AgentDB":
        """Alias for agent_db()."""
        return self.agent_db(agent_id, project=project)


class Bank:
    """Scoped memory — every op carries bank_id in meta.scope.

    Like Hindsight's bankId, but for synapse.
    """

    def __init__(self, client: Client, bank_id: str):
        self.client = client
        self.bank_id = bank_id

    def _scope(self, extra: Optional[Dict] = None) -> Dict:
        m = {"scope": f"bank/{_scope_component(self.bank_id)}"}
        if extra:
            m.update(extra)
        return m

    def retain(self, text: str, title: Optional[str] = None,
               meta: Optional[Dict] = None) -> int:
        return self.client.put(text, title=title, meta=self._scope(meta))

    def recall(self, query: str, limit: int = 5) -> List[Dict]:
        return self.client.search_scoped_fusion(
            query,
            scope=f"bank/{_scope_component(self.bank_id)}",
            limit=limit,
            fetch_k=max(limit * 10, 50),
        )

    def reflect(self, limit: int = 20) -> List[Dict]:
        docs = self.client.timeline(limit=limit * 3)
        return [d for d in docs
                if (d.get("meta") or {}).get("scope") == f"bank/{_scope_component(self.bank_id)}"][:limit]


def estimate_tokens(text: str) -> int:
    """Cheap tokenizer-independent estimate for context budgeting."""
    if not text:
        return 0
    return max(1, (len(text) + 3) // 4)


def _compact_text(text: str, max_chars: int) -> str:
    collapsed = re.sub(r"\s+", " ", text or "").strip()
    if len(collapsed) <= max_chars:
        return collapsed
    return collapsed[:max_chars].rstrip() + "..."


def _scope_component(value: str) -> str:
    return str(value).replace("%", "%25").replace("/", "%2F")


def _safe_id(value: Any) -> Optional[int]:
    try:
        return int(value)
    except (TypeError, ValueError):
        return None


class AgentDB:
    """Agent-native memory database facade.

    Design goal: make the public API reflect the winning architecture:
    scoped hot recall first, compact index results second, full observation
    hydration only when useful, and feedback logging for future learned rerank.
    """

    schema = "synapse.agentdb.v1"

    def __init__(self, client: Client, agent_id: str,
                 project: Optional[str] = None):
        if not agent_id:
            raise ValueError("agent_id must be non-empty")
        self.client = client
        self.agent_id = agent_id
        self.project = project or None
        agent_scope_id = _scope_component(agent_id)
        self.scope = (
            f"agent/{agent_scope_id}"
            if self.project is None
            else f"agent/{_scope_component(self.project)}/{agent_scope_id}"
        )

    def _meta(self, kind: str, extra: Optional[Dict[str, Any]] = None) -> Dict[str, Any]:
        meta: Dict[str, Any] = {
            "schema": self.schema,
            "scope": self.scope,
            "agent_id": self.agent_id,
            "kind": kind,
        }
        if self.project:
            meta["project"] = self.project
        if extra:
            meta.update(extra)
        return meta

    def observe(self, text: str, title: Optional[str] = None,
                kind: str = "observation",
                tags: Optional[List[str]] = None,
                source_uri: Optional[str] = None,
                confidence: Optional[float] = None,
                valid_from: Optional[Union[int, float, str]] = None,
                valid_until: Optional[Union[int, float, str]] = None,
                meta: Optional[Dict[str, Any]] = None,
                embed: bool = True) -> int:
        """Store an agent observation with lifecycle/freshness metadata."""
        extra = dict(meta or {})
        if tags:
            extra["tags"] = list(tags)
        if source_uri:
            extra["source_uri"] = source_uri
        if confidence is not None:
            extra["confidence"] = float(confidence)
        if valid_from is not None:
            extra["valid_from"] = valid_from
        if valid_until is not None:
            extra["valid_until"] = valid_until
        return self.client.put(
            text,
            title=title,
            uri=source_uri,
            meta=self._meta(kind, extra),
            embed=embed,
        )

    remember = observe

    @staticmethod
    def _freshness(meta: Dict[str, Any]) -> str:
        valid_until = meta.get("valid_until")
        if isinstance(valid_until, (int, float)) and valid_until < time.time():
            return "stale"
        return "current" if valid_until is not None or meta.get("source_uri") else "unknown"

    def search_index(self, query: str, limit: int = 8,
                     snippet_chars: int = 240) -> List[Dict[str, Any]]:
        """Return compact search hits designed for first-pass agent context."""
        hits = self.client.search_scoped_fusion(
            query,
            scope=self.scope,
            limit=limit,
            fetch_k=max(limit * 12, 80),
            modes=("lex", "hybrid"),
        )
        index = []
        for rank, hit in enumerate(hits, 1):
            doc_id = _safe_id(hit.get("id"))
            if doc_id is None:
                continue
            meta = hit.get("meta") or {}
            if meta.get("scope") and meta.get("scope") != self.scope:
                continue
            text = str(hit.get("text") or "")
            index.append({
                "rank": rank,
                "id": doc_id,
                "score": float(hit.get("score") or 0.0),
                "title": hit.get("title"),
                "uri": hit.get("uri"),
                "kind": meta.get("kind") or meta.get("type") or "memory",
                "tags": meta.get("tags") or [],
                "freshness": self._freshness(meta),
                "confidence": meta.get("confidence"),
                "token_estimate": estimate_tokens(text),
                "snippet": _compact_text(text, snippet_chars),
                "meta": meta,
            })
        return index

    def get_observations(self, ids: List[Union[int, str]],
                         max_chars: Optional[int] = None) -> List[Dict[str, Any]]:
        """Hydrate full observations by id, restricted to this agent scope."""
        docs = self.client.get_docs(ids, max_chars=max_chars)
        out = []
        for doc in docs:
            meta = doc.get("meta") or {}
            if meta.get("scope") != self.scope:
                continue
            doc["kind"] = meta.get("kind") or meta.get("type") or "memory"
            doc["freshness"] = self._freshness(meta)
            out.append(doc)
        return out

    def timeline(self, limit: int = 20, offset: int = 0,
                 kind: Optional[str] = None) -> List[Dict[str, Any]]:
        """Recent scoped memories, optionally filtered by observation kind."""
        return self.client.timeline_scoped(self.scope, limit=limit, offset=offset, kind=kind)

    def context_pack(self, query: str, token_budget: int = 800,
                     index_k: int = 8, full_k: int = 3,
                     snippet_chars: int = 220) -> Dict[str, Any]:
        """Build an XML context block optimized for coding-agent prompts.

        The pack contains a compact index for breadth and only a few hydrated
        observations for depth. This is the core token-saving pattern that lets
        agents recall more accurately without flooding the prompt.
        """
        raw_index = self.search_index(query, limit=index_k, snippet_chars=snippet_chars)
        index = []
        used_tokens = estimate_tokens(query) + 64
        for item in raw_index:
            item_tokens = estimate_tokens(item["snippet"]) + 18
            if index and used_tokens + item_tokens > token_budget:
                break
            if used_tokens + item_tokens > token_budget:
                continue
            index.append(item)
            used_tokens += item_tokens
        all_docs = self.get_observations([item["id"] for item in index])
        docs_by_id = {doc["id"]: doc for doc in all_docs}

        selected = []
        for item in index[:full_k]:
            doc = docs_by_id.get(item["id"])
            if not doc:
                continue
            doc_tokens = int(doc.get("token_estimate") or estimate_tokens(doc.get("text", "")))
            if used_tokens + doc_tokens > token_budget:
                remaining = token_budget - used_tokens
                if remaining <= 8:
                    continue
                clipped = dict(doc)
                max_chars = max(0, (remaining - 4) * 4)
                text = str(doc.get("text") or "")
                clipped["text"] = text[:max_chars].rstrip() + "\n[truncated]"
                clipped["truncated"] = True
                clipped["token_estimate"] = estimate_tokens(clipped["text"])
                if used_tokens + int(clipped["token_estimate"]) > token_budget:
                    continue
                selected.append(clipped)
                used_tokens += int(clipped["token_estimate"])
                continue
            selected.append(doc)
            used_tokens += doc_tokens

        naive_tokens = sum(
            int(doc.get("token_estimate") or estimate_tokens(doc.get("text", "")))
            for doc in all_docs
        )
        saved = max(0, naive_tokens - used_tokens)
        savings_pct = round((saved / naive_tokens) * 100, 1) if naive_tokens else 0.0
        context = self._render_context(query, index, selected, token_budget, used_tokens)
        return {
            "schema": self.schema,
            "agent_id": self.agent_id,
            "project": self.project,
            "scope": self.scope,
            "query": query,
            "token_budget": token_budget,
            "estimated_tokens": used_tokens,
            "naive_full_recall_tokens": naive_tokens,
            "token_savings_pct": savings_pct,
            "index": index,
            "observations": selected,
            "context": context,
        }

    def feedback(self, query: str, hit_ids: List[Union[int, str]],
                 outcome: str, accepted: bool = True,
                 meta: Optional[Dict[str, Any]] = None) -> int:
        """Log recall outcome for learned routing/reranking."""
        clean_ids = [doc_id for doc_id in (_safe_id(v) for v in hit_ids) if doc_id is not None]
        payload = {
            "query": query,
            "hit_ids": clean_ids,
            "outcome": outcome,
            "accepted": bool(accepted),
            "ts": int(time.time()),
        }
        return self.client.put(
            json.dumps(payload, sort_keys=True),
            title=f"agent-feedback/{outcome}",
            meta=self._meta("feedback", meta),
            embed=False,
        )

    def _render_context(self, query: str, index: List[Dict[str, Any]],
                        observations: List[Dict[str, Any]],
                        token_budget: int, estimated_tokens: int) -> str:
        lines = [
            (
                f'<synapse_agent_context schema="{html.escape(self.schema)}" '
                f'agent="{html.escape(self.agent_id)}" '
                f'scope="{html.escape(self.scope)}" '
                f'token_budget="{token_budget}" estimated_tokens="{estimated_tokens}">'
            ),
            f"  <query>{html.escape(query)}</query>",
            "  <search_index>",
        ]
        for item in index:
            lines.append(
                "    "
                f'<hit id="{item["id"]}" rank="{item["rank"]}" '
                f'kind="{html.escape(str(item["kind"]))}" '
                f'freshness="{html.escape(str(item["freshness"]))}" '
                f'score="{item["score"]:.6f}">'
                f'<title>{html.escape(str(item.get("title") or ""))}</title>'
                f'<snippet>{html.escape(item.get("snippet") or "")}</snippet>'
                "</hit>"
            )
        lines.extend(["  </search_index>", "  <observations>"])
        for doc in observations:
            meta = doc.get("meta") or {}
            lines.append(
                "    "
                f'<observation id="{doc["id"]}" '
                f'kind="{html.escape(str(doc.get("kind") or "memory"))}" '
                f'freshness="{html.escape(str(doc.get("freshness") or "unknown"))}">'
            )
            lines.append(f"      <title>{html.escape(str(doc.get('title') or ''))}</title>")
            if meta.get("source_uri"):
                lines.append(f"      <source>{html.escape(str(meta['source_uri']))}</source>")
            lines.append(f"      <text>{html.escape(str(doc.get('text') or ''))}</text>")
            lines.append("    </observation>")
        lines.extend(["  </observations>", "</synapse_agent_context>"])
        return "\n".join(lines)
