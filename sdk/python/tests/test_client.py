import unittest

from synapse_memory.client import Client


class FakeClient(Client):
    def __init__(self):
        super().__init__(sock_path="/tmp/unused")
        self.requests = []

    def _call(self, req, timeout=None):
        self.requests.append(req)
        if req["op"] == "BatchSearch":
            return {
                "BatchHits": [
                    [
                        {"id": 1, "text": "out of scope", "score": 1.0},
                        {"id": 2, "text": "Phase 1 shipped", "score": 0.7},
                    ],
                    [
                        {"id": 2, "text": "Phase 1 shipped", "score": 0.9},
                    ],
                ]
            }
        if req["op"] == "Sql":
            return {
                "Rows": {
                    "cols": ["id", "meta"],
                    "rows": [
                        [1, '{"scope":"bank/other"}'],
                        [2, '{"scope":"bank/project"}'],
                    ],
                }
            }
        raise AssertionError(f"unexpected request: {req}")


class FakeAgentClient(Client):
    def __init__(self):
        super().__init__(sock_path="/tmp/unused")
        self.puts = []
        self.docs = {
            10: {
                "id": 10,
                "uri": "file:///app/a.py",
                "title": "Router decision",
                "text": "Use scoped fusion for agent recall because it keeps unrelated memories out.",
                "meta": {
                    "schema": "synapse.agentdb.v1",
                    "scope": "agent/project/coder",
                    "agent_id": "coder",
                    "project": "project",
                    "kind": "decision",
                    "tags": ["recall"],
                    "source_uri": "file:///app/a.py",
                },
                "ts": 100,
            },
            11: {
                "id": 11,
                "uri": "file:///app/b.py",
                "title": "Slow path",
                "text": "Batch search removes socket roundtrip overhead for hook chains.",
                "meta": {
                    "schema": "synapse.agentdb.v1",
                    "scope": "agent/project/coder",
                    "agent_id": "coder",
                    "project": "project",
                    "kind": "observation",
                },
                "ts": 101,
            },
            12: {
                "id": 12,
                "uri": "file:///other",
                "title": "Other scope",
                "text": "Must not leak into this agent context.",
                "meta": {
                    "schema": "synapse.agentdb.v1",
                    "scope": "agent/other/coder",
                    "agent_id": "coder",
                    "kind": "observation",
                },
                "ts": 102,
            },
        }

    def search_scoped_fusion(self, query, scope, limit=5, fetch_k=None,
                             modes=("lex", "hybrid"), embed_query=True):
        return [
            {**self.docs[10], "score": 0.99},
            {**self.docs[11], "score": 0.80},
            {**self.docs[12], "score": 0.70},
        ][:limit]

    def sql(self, query, params=None):
        params = params or []
        if "WHERE id IN" in query:
            out = []
            for doc_id in params:
                doc = self.docs[int(doc_id)]
                out.append({
                    **doc,
                    "meta": __import__("json").dumps(doc["meta"]),
                })
            return out
        if "json_extract(meta, '$.scope') = ?" in query:
            scope = params[0]
            kind = params[1] if len(params) > 3 else None
            docs = []
            for doc in sorted(self.docs.values(), key=lambda item: item["ts"], reverse=True):
                meta = doc["meta"]
                if meta.get("scope") != scope:
                    continue
                if kind and meta.get("kind") != kind and meta.get("type") != kind:
                    continue
                docs.append({**doc, "meta": __import__("json").dumps(meta)})
            return docs
        raise AssertionError(f"unexpected sql: {query} {params}")

    def put(self, text, title=None, uri=None, meta=None, embed=True):
        self.puts.append({
            "text": text,
            "title": title,
            "uri": uri,
            "meta": meta,
            "embed": embed,
        })
        return 999


class LargeScopeClient(Client):
    def __init__(self):
        super().__init__(sock_path="/tmp/unused")

    def _scope_candidates(self, scope, limit):
        return [
            {
                "id": i,
                "title": f"recent noise {i}",
                "text": "recent unrelated filler row",
                "meta": {"scope": scope},
                "score": 0.0,
            }
            for i in range(1000, 1000 + limit)
        ]

    def _scope_query_candidates(self, query, scope, limit):
        return [
            {
                "id": 42,
                "title": "old exact target",
                "text": "progressive disclosure saves context tokens for agent memory",
                "meta": {"scope": scope},
                "score": 0.0,
            }
        ]


class ClientTest(unittest.TestCase):
    def test_batch_search_shapes_daemon_request_and_hydrates_meta(self):
        client = FakeClient()

        batches = client.batch_search(
            [
                "phase shipped",
                {"query": "phase", "mode": "lex", "limit": 3, "embed_query": False},
            ],
            include_meta=True,
        )

        self.assertEqual(client.requests[0]["op"], "BatchSearch")
        self.assertEqual(client.requests[0]["args"]["queries"][0]["mode"], "Hybrid")
        self.assertEqual(client.requests[0]["args"]["queries"][1]["mode"], "Lex")
        self.assertFalse(client.requests[0]["args"]["queries"][1]["embed_query"])
        self.assertEqual(batches[0][0]["meta"]["scope"], "bank/other")
        self.assertEqual(batches[0][1]["meta"]["scope"], "bank/project")

    def test_bank_recall_filters_by_scope(self):
        client = FakeClient()

        hits = client.bank("project").recall("phase shipped", limit=5)

        self.assertEqual([h["id"] for h in hits], [2])
        self.assertEqual(hits[0]["meta"]["scope"], "bank/project")

    def test_agent_context_pack_uses_compact_index_and_scoped_full_docs(self):
        agent = FakeAgentClient().agent("coder", project="project")

        pack = agent.context_pack("how should recall route?", token_budget=240)

        self.assertEqual(pack["schema"], "synapse.agentdb.v1")
        self.assertEqual(pack["scope"], "agent/project/coder")
        self.assertEqual([item["id"] for item in pack["index"]], [10, 11])
        self.assertEqual([doc["id"] for doc in pack["observations"]], [10, 11])
        self.assertIn("<search_index>", pack["context"])
        self.assertIn("<observations>", pack["context"])
        self.assertGreaterEqual(pack["token_savings_pct"], 0.0)

    def test_agent_scope_escapes_slashes_to_avoid_cross_agent_leaks(self):
        client = FakeAgentClient()

        a = client.agent("a/b", project="p")
        b = client.agent("b", project="p/a")

        self.assertEqual(a.scope, "agent/p/a%2Fb")
        self.assertEqual(b.scope, "agent/p%2Fa/b")
        self.assertNotEqual(a.scope, b.scope)

    def test_scoped_fusion_finds_older_query_match_beyond_recent_window(self):
        client = LargeScopeClient()

        hits = client.search_scoped_fusion(
            "how should agent memory save context tokens",
            scope="agent/project/coder",
            limit=5,
            fetch_k=80,
        )

        self.assertEqual(hits[0]["id"], 42)

    def test_context_pack_does_not_hydrate_full_doc_over_budget(self):
        agent = FakeAgentClient().agent("coder", project="project")

        pack = agent.context_pack("how should recall route?", token_budget=100)

        self.assertEqual(pack["observations"], [])
        self.assertLessEqual(pack["estimated_tokens"], 100)

    def test_agent_get_observations_and_timeline_filter_scope(self):
        agent = FakeAgentClient().agent("coder", project="project")

        docs = agent.get_observations([12, 11, 10])
        timeline = agent.timeline(kind="decision")

        self.assertEqual([doc["id"] for doc in docs], [11, 10])
        self.assertEqual([doc["id"] for doc in timeline], [10])

    def test_agent_feedback_logs_learning_signal_without_embedding(self):
        client = FakeAgentClient()
        agent = client.agent("coder", project="project")

        feedback_id = agent.feedback("route recall", [10, "11"], outcome="accepted")

        self.assertEqual(feedback_id, 999)
        self.assertEqual(client.puts[0]["title"], "agent-feedback/accepted")
        self.assertFalse(client.puts[0]["embed"])
        self.assertEqual(client.puts[0]["meta"]["kind"], "feedback")


if __name__ == "__main__":
    unittest.main()
