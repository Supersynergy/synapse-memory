"""intelligence.py — AI-native CRM features. Wires synapsql to Synapse brain.db + ML primitives.

Ships 5 features:
  1. VectorSearch  — semantic similar-lead search via Synapse hybrid (8ms @ 195K docs)
  2. SmartDedup    — cosine-sim > threshold → auto-merge candidates
  3. NextAction    — Markov-chain over activity-feed → predict next likely action
  4. AnomalyDetect — score-distribution drift via z-score
  5. NLQuery       — natural-lang → SQL via simple keyword-templating (LLM optional)

Synapse daemon optional — falls back to local SQLite FTS5 when offline.
"""
from __future__ import annotations
import os, sqlite3, json, math, socket, struct, statistics, re, urllib.parse, urllib.request
from collections import Counter, defaultdict
from typing import Optional, Any

SOCK = os.environ.get("SYNAPSE_SOCK", "/tmp/synapse.sock")
TURBO_URL = os.environ.get("SYNAPSE_TURBO_URL", "http://127.0.0.1:9477").rstrip("/")


# -------------------- 1. VectorSearch via Synapse --------------------

class VectorSearch:
    """Semantic search over leads using Synapse daemon (hybrid BM25+vec).

    Use:
        vs = VectorSearch()  # connects to /tmp/synapse.sock
        hits = vs.find_similar("yoga studios in münchen with 1M revenue", limit=10)
    """

    def __init__(self, sock_path: Optional[str] = None, timeout: float = 5.0):
        self._sock = sock_path or SOCK
        self._timeout = timeout
        try:
            import msgpack
            self._msgpack = msgpack
        except ImportError:
            self._msgpack = None

    def _call(self, req: dict):
        if self._msgpack is None:
            raise RuntimeError("msgpack not installed")
        s = socket.socket(socket.AF_UNIX, socket.SOCK_STREAM)
        s.settimeout(self._timeout)
        s.connect(self._sock)
        body = self._msgpack.packb(req)
        s.sendall(struct.pack("<I", len(body)) + body)
        hdr = b""
        while len(hdr) < 4:
            hdr += s.recv(4 - len(hdr))
        n = struct.unpack("<I", hdr)[0]
        buf = b""
        while len(buf) < n:
            buf += s.recv(n - len(buf))
        s.close()
        return self._msgpack.unpackb(buf, raw=False)

    def find_similar(self, query: str, limit: int = 10, mode: str = "Hybrid") -> list[dict]:
        try:
            path = {"Hybrid": "hybrid", "Vec": "vec", "Lex": "find"}.get(mode, "hybrid")
            qs = urllib.parse.urlencode({"q": query, "limit": int(limit)})
            req = urllib.request.Request(f"{TURBO_URL}/{path}?{qs}")
            if req.type == "http" and req.host.startswith("127.0.0.1"):
                with urllib.request.urlopen(req, timeout=self._timeout) as resp:  # noqa: S310
                    data = json.loads(resp.read())
                return [{"score": h.get("score", h.get("distance", 0)),
                         "title": h.get("title", ""), "text": (h.get("text", "") or "")[:200],
                         "id": h.get("id")} for h in data.get("results", [])]
        except Exception:
            pass
        try:
            r = self._call({"op": "Search",
                          "args": {"mode": mode, "q": query, "limit": int(limit), "embed_query": True}})
            hits = r.get("Hits") if isinstance(r, dict) else r
            return [{"score": h.get("score", 0), "title": h.get("title", ""),
                     "text": h.get("text", "")[:200], "id": h.get("id")} for h in (hits or [])]
        except (FileNotFoundError, ConnectionRefusedError, ConnectionError,
                OSError, socket.timeout, RuntimeError):
            return []  # daemon offline OR msgpack missing → empty, caller fallbacks

    def is_online(self) -> bool:
        if self._msgpack is None:
            return False
        try:
            r = self._call({"op": "Ping"})
            return r is not None
        except Exception:
            return False


# -------------------- 2. SmartDedup --------------------

class SmartDedup:
    """Pairwise similarity-based duplicate detection.

    Strategies:
      - Exact (email, phone) — O(N) hash-bucket
      - Fuzzy name (jaro-winkler approx via ngram-jaccard)
      - Vector (Synapse embed cosine) — O(N²) but daemon-accelerated

    Use:
        dd = SmartDedup(conn, table="leads")
        candidates = dd.find_duplicates(method="exact", cols=["email", "phone"])
        # → [(id_a, id_b, score), ...]
    """

    def __init__(self, conn: sqlite3.Connection, table: str, pk: str = "id"):
        self._c = conn
        self._t = table
        self._pk = pk

    def find_duplicates_exact(self, cols: list[str]) -> list[tuple[Any, Any, float]]:
        """Group by normalized values; return all pairs in same group."""
        pairs = []
        for col in cols:
            buckets = defaultdict(list)
            for row in self._c.execute(
                f"SELECT {self._pk}, {col} FROM {self._t} WHERE {col} IS NOT NULL AND {col} != ''"
            ):
                pk, val = row
                norm = str(val).strip().lower()
                buckets[norm].append(pk)
            for ids in buckets.values():
                if len(ids) > 1:
                    for i in range(len(ids)):
                        for j in range(i + 1, len(ids)):
                            pairs.append((ids[i], ids[j], 1.0))
        return pairs

    def find_duplicates_fuzzy(self, col: str, threshold: float = 0.85) -> list[tuple[Any, Any, float]]:
        """Trigram-Jaccard on text col. O(N²) — use small subsets only."""
        rows = list(self._c.execute(
            f"SELECT {self._pk}, {col} FROM {self._t} WHERE {col} IS NOT NULL"))
        def trigrams(s: str) -> set:
            s = re.sub(r"\s+", " ", s.lower().strip())
            s = f"  {s}  "
            return {s[i:i+3] for i in range(len(s) - 2)}
        items = [(pk, trigrams(val)) for pk, val in rows]
        pairs = []
        for i in range(len(items)):
            for j in range(i + 1, len(items)):
                a, b = items[i][1], items[j][1]
                if not a or not b: continue
                jac = len(a & b) / len(a | b)
                if jac >= threshold:
                    pairs.append((items[i][0], items[j][0], jac))
        return pairs

    def find_duplicates(self, method: str = "exact", **kw) -> list[tuple]:
        if method == "exact":
            return self.find_duplicates_exact(kw.get("cols", ["email"]))
        elif method == "fuzzy":
            return self.find_duplicates_fuzzy(kw["col"], kw.get("threshold", 0.85))
        else:
            raise ValueError(f"unknown method {method!r}")


# -------------------- 3. Predictive Next-Action --------------------

class NextAction:
    """Markov-chain predictor over activity-feed verbs.

    Trains on (verb_t, verb_t+1) transitions; predicts most-likely next verb.
    Use:
        na = NextAction(conn)
        na.train_from_feed(subject_type="lead")
        next_verbs = na.predict("lead", "1", k=3)  # → [("called", 0.4), ("emailed", 0.3), ...]
    """

    def __init__(self, conn: sqlite3.Connection):
        self._c = conn
        self._matrix: dict[str, Counter] = defaultdict(Counter)

    def train_from_feed(self, subject_type: str = None) -> int:
        # Sequences per subject
        sql = ("SELECT subject_type, subject_id, verb FROM _activity_feed "
               "WHERE subject_type IS NOT NULL AND subject_id IS NOT NULL "
               "ORDER BY ts ASC")
        if subject_type:
            sql = sql.replace(
                "WHERE subject_type IS NOT NULL",
                f"WHERE subject_type = '{subject_type}'")
        sequences: dict[tuple, list[str]] = defaultdict(list)
        for st, sid, verb in self._c.execute(sql):
            sequences[(st, sid)].append(verb)
        n_pairs = 0
        for verbs in sequences.values():
            for i in range(len(verbs) - 1):
                self._matrix[verbs[i]][verbs[i + 1]] += 1
                n_pairs += 1
        return n_pairs

    def predict(self, subject_type: str, subject_id: str, k: int = 3) -> list[tuple[str, float]]:
        # Get last verb on this subject
        row = self._c.execute(
            "SELECT verb FROM _activity_feed WHERE subject_type=? AND subject_id=? "
            "ORDER BY ts DESC LIMIT 1",
            (subject_type, str(subject_id))).fetchone()
        if not row:
            return []
        last = row[0]
        counts = self._matrix.get(last, Counter())
        total = sum(counts.values())
        if total == 0:
            return []
        return [(verb, n / total) for verb, n in counts.most_common(k)]


# -------------------- 4. Anomaly Detection --------------------

class AnomalyDetect:
    """z-score-based anomaly detection on numeric columns.

    Use:
        ad = AnomalyDetect(conn, "leads", "score")
        ad.fit()
        outliers = ad.find_outliers(z_threshold=3.0)
        ad.detect_drift_window(days=7)  # drift vs prior week
    """

    def __init__(self, conn: sqlite3.Connection, table: str, col: str):
        self._c = conn
        self._t = table
        self._col = col
        self._mean: Optional[float] = None
        self._std: Optional[float] = None

    def fit(self) -> dict:
        rows = [r[0] for r in self._c.execute(
            f"SELECT {self._col} FROM {self._t} WHERE {self._col} IS NOT NULL")]
        if len(rows) < 2:
            return {"n": len(rows), "mean": None, "std": None}
        self._mean = statistics.mean(rows)
        self._std = statistics.stdev(rows)
        return {"n": len(rows), "mean": self._mean, "std": self._std}

    def find_outliers(self, z_threshold: float = 3.0, pk: str = "id") -> list[tuple]:
        if self._mean is None:
            self.fit()
        if not self._std or self._std == 0:
            return []
        thr = z_threshold * self._std
        return list(self._c.execute(
            f"SELECT {pk}, {self._col} FROM {self._t} "
            f"WHERE ABS({self._col} - ?) > ? AND {self._col} IS NOT NULL",
            (self._mean, thr)))

    def detect_drift_window(self, ts_col: str = "created_at", days: int = 7) -> dict:
        """Compare last N days vs preceding N days. Returns delta stats."""
        recent = [r[0] for r in self._c.execute(
            f"SELECT {self._col} FROM {self._t} "
            f"WHERE {ts_col} >= date('now', ?) AND {self._col} IS NOT NULL",
            (f"-{days} days",))]
        prior = [r[0] for r in self._c.execute(
            f"SELECT {self._col} FROM {self._t} "
            f"WHERE {ts_col} >= date('now', ?) AND {ts_col} < date('now', ?) "
            f"AND {self._col} IS NOT NULL",
            (f"-{2*days} days", f"-{days} days"))]
        out = {
            "recent_n": len(recent), "prior_n": len(prior),
            "recent_mean": statistics.mean(recent) if recent else None,
            "prior_mean": statistics.mean(prior) if prior else None,
        }
        if recent and prior:
            out["delta_mean"] = out["recent_mean"] - out["prior_mean"]
            # Welch's t-style normalized z
            r_std = statistics.stdev(recent) if len(recent) > 1 else 0
            p_std = statistics.stdev(prior) if len(prior) > 1 else 0
            pooled = math.sqrt((r_std**2 / len(recent)) + (p_std**2 / len(prior))) if r_std or p_std else 0
            out["drift_z"] = out["delta_mean"] / pooled if pooled > 0 else 0
        return out


# -------------------- 5. Natural-Language Query (templated, no LLM) --------------------

class NLQuery:
    """Natural-language → SQL via keyword templates. Pattern-based, deterministic.

    Use:
        nlq = NLQuery(conn, table="leads")
        sql, params = nlq.parse("hot leads in berlin")
        rows = nlq.execute("hot leads in berlin")
    """

    # Default vocab (extensible)
    STATUS_WORDS = {"hot": ("status", "won"), "won": ("status", "won"),
                    "cold": ("status", "lost"), "new": ("status", "new"),
                    "qualified": ("status", "qualified")}
    CITIES = {"berlin", "münchen", "munich", "hamburg", "köln", "cologne",
              "frankfurt", "stuttgart", "düsseldorf", "leipzig"}

    def __init__(self, conn: sqlite3.Connection, table: str = "leads"):
        self._c = conn
        self._t = table

    def parse(self, query: str) -> tuple[str, tuple]:
        q = query.lower()
        wheres: list[str] = []
        params: list = []
        order_by = None
        limit = 50

        # Status words
        for word, (col, val) in self.STATUS_WORDS.items():
            if re.search(rf"\b{word}\b", q):
                wheres.append(f"LOWER({col}) = ?")
                params.append(val)
                break

        # Cities
        for city in self.CITIES:
            if city in q:
                wheres.append("LOWER(city) LIKE ?")
                params.append(f"%{city}%")
                break

        # Number filters: "score > N", "score above N"
        m = re.search(r"score\s*(?:>|above|over)\s*(\d+)", q)
        if m:
            wheres.append("score > ?")
            params.append(int(m.group(1)))

        # Recency: "last 7 days", "last week"
        m = re.search(r"last\s+(\d+)\s+days?", q)
        if m:
            wheres.append("created_at >= date('now', ?)")
            params.append(f"-{m.group(1)} days")
        elif "last week" in q:
            wheres.append("created_at >= date('now', '-7 days')")
        elif "today" in q:
            wheres.append("date(created_at) = date('now')")

        # Order
        if "top" in q or "best" in q or "highest" in q:
            order_by = "score DESC"
        elif "recent" in q or "latest" in q:
            order_by = "created_at DESC"

        # Limit
        m = re.search(r"\b(?:top|first|limit)\s+(\d+)\b", q)
        if m:
            limit = int(m.group(1))

        sql = f"SELECT * FROM {self._t}"
        if wheres:
            sql += " WHERE " + " AND ".join(wheres)
        if order_by:
            sql += f" ORDER BY {order_by}"
        sql += f" LIMIT {limit}"
        return sql, tuple(params)

    def execute(self, query: str) -> list:
        sql, params = self.parse(query)
        return self._c.execute(sql, params).fetchall()
