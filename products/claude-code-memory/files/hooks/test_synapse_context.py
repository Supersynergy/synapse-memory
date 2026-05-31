#!/usr/bin/env python3
import json
import os
import sys
import tempfile
import unittest
from pathlib import Path
from unittest.mock import patch

sys.path.insert(0, str(Path(__file__).resolve().parent))

import synapse_context as sc


FAKE_LINES = [
    "0.021\tdecision/autolearn\tWe verified autolearn and superml routing with token budget recall context optimizer.",
    "0.020\tdecision/autolearn\tWe verified autolearn and superml routing with token budget recall context optimizer.",
    "0.018\tbench/debug\tCargo test failed in synapsed because stats cache overcounted deduped puts; fixed by max_doc_id.",
    "0.016\ttelepathy/recent\t[telepathy][abcd1234][synapse][reply] unrelated chatter from another session.",
    "0.014\tarchitecture/context\tContext packing should center snippets around recall terms and enforce a hard token budget.",
]


class SynapseContextTests(unittest.TestCase):
    def setUp(self):
        self.env_patch = patch.dict(os.environ, {"SYNAPSE_CONTEXT_NO_LEARN": "1", "SYNAPSE_FRESH_CONTEXT": "0"})
        self.env_patch.start()

    def tearDown(self):
        self.env_patch.stop()

    def test_parse_hook_json_and_raw(self):
        prompt, sid, cwd = sc.parse_hook_input(
            json.dumps({"prompt": "fix failing tests", "session_id": "abc123", "cwd": "/tmp/proj"})
        )
        self.assertEqual(prompt, "fix failing tests")
        self.assertEqual(sid, "abc123")
        self.assertEqual(cwd, "/tmp/proj")
        self.assertEqual(sc.parse_hook_input("raw prompt")[0], "raw prompt")

    def test_short_prompt_skips(self):
        self.assertIsNone(sc.classify_prompt("+", "prompt", None))
        self.assertIsNone(sc.classify_prompt("ok", "prompt", None))

    def test_debug_policy_gets_larger_budget(self):
        p = sc.classify_prompt("cargo test failed with panic in stats cache", "prompt", None)
        self.assertIsNotNone(p)
        self.assertEqual(p.name, "debug")
        self.assertGreaterEqual(p.max_tokens, 520)

    def test_pack_respects_budget_and_dedupes(self):
        policy = sc.Policy("optimize", "autolearn superml recall token context", 8, 145, 0.006, 120, 10)
        packed, used, _naive = sc.pack_hits(FAKE_LINES, policy)
        self.assertLessEqual(used, policy.max_tokens)
        self.assertEqual(len({h.title for h in packed}), len(packed))
        self.assertEqual(sum(1 for h in packed if h.title == "decision/autolearn"), 1)

    def test_snippet_centers_query_term(self):
        text = "prefix " * 60 + "needle important recall context " + "suffix " * 60
        snippet = sc.centered_snippet(text, ["needle"], 120)
        self.assertIn("needle important recall", snippet)
        self.assertLessEqual(len(snippet), 126)

    def test_end_to_end_render_uses_fake_synx(self):
        with patch.object(sc, "fetch_hits", return_value=FAKE_LINES):
            out = sc.build_context(
                json.dumps({"prompt": "bitte autolearn superml token recall context optimieren"}),
                "prompt",
            )
        self.assertIn("<synapse_context", out)
        self.assertIn("class=\"optimize\"", out)
        self.assertIn("saved_vs_candidates", out)

    def test_recall_plan_adds_optimizer_perspectives(self):
        policy = sc.classify_prompt("optimize recall token benchmark latency", "prompt", "synapse")
        self.assertIsNotNone(policy)
        plan = sc.recall_plan(policy)
        names = {q.name for q in plan}
        self.assertIn("primary", names)
        self.assertIn("bench", names)
        self.assertIn("decision", names)
        self.assertGreater(len(plan), 1)

    def test_fetch_hits_parses_batch_jsonl(self):
        policy = sc.Policy("optimize", "recall benchmark latency", 4, 300, 0.0, 120, 5)
        plan = sc.recall_plan(policy)
        rows = []
        for q in plan[:2]:
            rows.append(
                json.dumps(
                    {
                        "q": q.query,
                        "response": {
                            "Hits": [
                                {
                                    "score": 0.02,
                                    "title": f"{q.name}/title",
                                    "text": f"{q.name} verified recall benchmark latency",
                                }
                            ]
                        },
                    }
                )
            )

        class Proc:
            returncode = 0
            stdout = "\n".join(rows)

        with patch.object(sc, "synx_bin", return_value="synx"), patch.object(sc.subprocess, "run", return_value=Proc()):
            lines = sc.fetch_hits(policy)

        self.assertTrue(lines)
        parsed = [sc.parse_hit(line) for line in lines]
        self.assertTrue(all(item is not None for item in parsed))
        self.assertIn("primary", {item[0] for item in parsed if item})
        self.assertIn("bench", {item[0] for item in parsed if item})

    def test_learning_logs_and_rewards_recent_event(self):
        with tempfile.TemporaryDirectory() as tmp:
            self.env_patch.stop()
            try:
                with patch.dict(
                    os.environ,
                    {"SYNAPSE_RECALL_LEARN_DB": str(Path(tmp) / "learn.db")},
                    clear=False,
                ):
                    with patch.object(sc, "fetch_hits", return_value=FAKE_LINES):
                        out = sc.build_context(
                            json.dumps(
                                {
                                    "prompt": "bitte autolearn superml token recall context optimieren",
                                    "session_id": "sess1",
                                    "cwd": "/tmp/synapse",
                                }
                            ),
                            "prompt",
                        )
                    self.assertIn("<synapse_context", out)
                    rewarded = sc.reward_recent("verified tests passed and implementation done", project="synapse", session_id="sess1")
                    self.assertEqual(rewarded, 1)
                    stats = json.loads(sc.learn_stats())
                    self.assertEqual(stats["events"]["count"], 1)
                    self.assertTrue(any(row["rewards"] >= 1 for row in stats["bandit"]))
            finally:
                self.env_patch.start()

    def test_learned_weight_moves_rewarded_perspective(self):
        with tempfile.TemporaryDirectory() as tmp:
            self.env_patch.stop()
            try:
                with patch.dict(
                    os.environ,
                    {"SYNAPSE_RECALL_LEARN_DB": str(Path(tmp) / "learn.db")},
                    clear=False,
                ):
                    for _ in range(30):
                        sc.update_perspective_reward("optimize", ["bench"], 1.0, strength=1.0)
                        sc.update_perspective_reward("optimize", ["code"], 0.0, strength=1.0)
                    policy = sc.classify_prompt("optimize recall token benchmark latency", "prompt", "synapse")
                    self.assertIsNotNone(policy)
                    plan = {q.name: q.weight for q in sc.recall_plan(policy)}
                    self.assertGreater(plan["bench"], 1.14)
                    self.assertLess(plan["code"], 1.02)
            finally:
                self.env_patch.start()

    def test_fresh_context_reads_manifest_and_marks_registry_slip(self):
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            (root / "package.json").write_text(
                json.dumps({"dependencies": {"left-pad": "1.0.0", "langchain": "0.3.0"}}),
                encoding="utf-8",
            )
            env = {
                "SYNAPSE_FRESH_CONTEXT": "1",
                "SYNAPSE_FRESH_CONTEXT_DB": str(root / "fresh.db"),
                "SYNAPSE_FRESH_NATIVE": "0",
                "SYNAPSE_FRESH_TIMEOUT": "0.01",
            }
            with patch.dict(os.environ, env, clear=False), patch.object(
                sc,
                "fetch_registry_latest",
                return_value=sc.RegistryInfo("1.3.0", "test", "https://docs.example/left-pad"),
            ):
                out = sc.fresh_context_block("latest left-pad langchain", "prompt", str(root), "proj")

        self.assertIn("<fresh_context", out)
        self.assertIn("npm:left-pad", out)
        self.assertIn("latest=1.3.0", out)
        self.assertIn("status=pinned_differs", out)
        self.assertIn("docs=https://www.npmjs.com/package/left-pad/v/1.0.0", out)
        self.assertIn("broken/avoid langchain", out)

    def test_build_context_can_return_fresh_only(self):
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            (root / "Cargo.toml").write_text(
                """
[package]
name = "x"
version = "0.1.0"
edition = "2021"

[dependencies]
serde = "1.0.0"
""".strip(),
                encoding="utf-8",
            )
            env = {
                "SYNAPSE_FRESH_CONTEXT": "1",
                "SYNAPSE_FRESH_CONTEXT_DB": str(root / "fresh.db"),
                "SYNAPSE_FRESH_NATIVE": "0",
                "SYNAPSE_FRESH_TIMEOUT": "0.01",
            }
            with patch.dict(os.environ, env, clear=False), patch.object(
                sc,
                "fetch_registry_latest",
                return_value=sc.RegistryInfo("1.0.999", "test", "https://docs.rs/serde/latest/"),
            ):
                out = sc.build_context(json.dumps({"prompt": "latest serde", "cwd": str(root)}), "prompt")

        self.assertIn("<fresh_context", out)
        self.assertIn("crates:serde", out)
        self.assertIn("latest=1.0.999", out)
        self.assertIn("docs=https://docs.rs/serde/1.0.0/", out)

    def test_edge_stack_context_for_omega_and_context_slippage(self):
        with patch.dict(os.environ, {"SYNAPSE_EDGE_CONTEXT": "1"}, clear=False):
            out = sc.edge_stack_context_block(
                "omega cortext context7 latest docs version slippage agent memory",
                "prompt",
            )

        self.assertIn("<edge_stack_context", out)
        self.assertIn("OMEGA Memory", out)
        self.assertIn(".tools/omega-memory", out)
        self.assertIn("resolved/local package versions", out)
        self.assertIn("spelling note", out)

    def test_build_context_can_return_edge_only(self):
        with patch.dict(os.environ, {"SYNAPSE_EDGE_CONTEXT": "1"}, clear=False):
            out = sc.build_context(
                json.dumps({"prompt": "omega-memory vs Context7 for agent memory"}),
                "prompt",
            )

        self.assertIn("<edge_stack_context", out)
        self.assertIn("Synapse remains the local-first hot path", out)


if __name__ == "__main__":
    unittest.main()
