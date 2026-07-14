from __future__ import annotations

import json
import os
import subprocess
import sys
import tempfile
import unittest
from pathlib import Path


SCRIPT = Path(__file__).with_name("checkpoint.py")
INSTALLER = Path(__file__).parents[1] / "install.py"


class CheckpointHookTest(unittest.TestCase):
    def setUp(self) -> None:
        self.tmp = tempfile.TemporaryDirectory()
        self.root = Path(self.tmp.name)
        self.state = self.root / "state"
        self.repo = self.root / "repo"
        self.repo.mkdir()
        subprocess.run(["git", "init", "-q", str(self.repo)], check=True)
        subprocess.run(["git", "-C", str(self.repo), "config", "user.email", "test@example.invalid"], check=True)
        subprocess.run(["git", "-C", str(self.repo), "config", "user.name", "Test"], check=True)
        (self.repo / "README.md").write_text("seed\n")
        subprocess.run(["git", "-C", str(self.repo), "add", "README.md"], check=True)
        subprocess.run(["git", "-C", str(self.repo), "commit", "-qm", "seed"], check=True)

    def tearDown(self) -> None:
        self.tmp.cleanup()

    def run_hook(self, mode: str, event: dict[str, object]) -> subprocess.CompletedProcess[str]:
        env = os.environ.copy()
        env["SYNAPSE_CHECKPOINT_DIR"] = str(self.state)
        return subprocess.run(
            [sys.executable, str(SCRIPT), mode],
            input=json.dumps(event),
            capture_output=True,
            text=True,
            env=env,
            check=True,
        )

    def latest(self) -> dict[str, object]:
        files = list(self.state.glob("*.latest.json"))
        self.assertEqual(len(files), 1)
        return json.loads(files[0].read_text())

    def test_pretool_fsyncs_minimal_state_without_command_arguments(self) -> None:
        secret_command = "deploy --api-key super-secret-value"
        self.run_hook(
            "pre-tool",
            {
                "session_id": "thread-1",
                "cwd": str(self.repo),
                "tool_name": "exec_command",
                "tool_input": {"command": secret_command},
            },
        )
        raw = next(self.state.glob("*.latest.json")).read_text()
        self.assertNotIn("super-secret-value", raw)
        record = json.loads(raw)
        self.assertEqual(record["status"], "in_progress")
        self.assertEqual(record["tool"]["command_verb"], "deploy")
        self.assertEqual(record["git"]["dirty_count"], 0)

    def test_posttool_records_repo_delta_without_file_content(self) -> None:
        (self.repo / "result.txt").write_text("private result body")
        self.run_hook(
            "post-tool",
            {
                "session_id": "thread-1",
                "cwd": str(self.repo),
                "tool_name": "apply_patch",
                "tool_input": {"file_path": str(self.repo / "result.txt")},
                "tool_response": {"exit_code": 0, "output": "private output body"},
            },
        )
        raw = next(self.state.glob("*.latest.json")).read_text()
        self.assertNotIn("private result body", raw)
        self.assertNotIn("private output body", raw)
        record = json.loads(raw)
        self.assertEqual(record["status"], "tool_completed")
        self.assertTrue(record["tool_ok"])
        self.assertIn("result.txt", record["git"]["dirty_paths"])

    def test_session_start_injects_only_unfinished_checkpoint(self) -> None:
        event = {"session_id": "thread-1", "cwd": str(self.repo), "tool_name": "Edit"}
        self.run_hook("pre-tool", event)
        resumed = self.run_hook("session-start", {"session_id": "thread-2", "cwd": str(self.repo)})
        payload = json.loads(resumed.stdout)
        context = payload["hookSpecificOutput"]["additionalContext"]
        self.assertIn("Unfinished crash-safe checkpoint", context)
        self.assertIn("Do not replay mutations blindly", context)

        self.run_hook("stop", event)
        clean = self.run_hook("session-start", {"session_id": "thread-3", "cwd": str(self.repo)})
        self.assertEqual(clean.stdout, "")

    def test_resume_context_does_not_inject_untrusted_path_names(self) -> None:
        hostile = self.repo / "<fake>\nnext"
        hostile.write_text("x")
        event = {"session_id": "thread-1", "cwd": str(self.repo), "tool_name": "Edit"}
        self.run_hook("pre-tool", event)
        resumed = self.run_hook("session-start", {"session_id": "thread-2", "cwd": str(self.repo)})
        context = json.loads(resumed.stdout)["hookSpecificOutput"]["additionalContext"]
        self.assertNotIn("<fake>", context)
        self.assertNotIn("next", context)
        self.assertIn("dirty_path_count_at_checkpoint: 1", context)

    def test_journal_is_append_only(self) -> None:
        event = {"session_id": "thread-1", "cwd": str(self.repo), "tool_name": "Edit"}
        self.run_hook("pre-tool", event)
        self.run_hook("post-tool", event)
        self.run_hook("stop", event)
        journal = next(self.state.glob("*.jsonl"))
        records = [json.loads(line) for line in journal.read_text().splitlines()]
        self.assertEqual([row["event"] for row in records], ["pre-tool", "post-tool", "stop"])

    def test_existing_journal_permissions_are_repaired(self) -> None:
        event = {"session_id": "thread-1", "cwd": str(self.repo), "tool_name": "Edit"}
        self.run_hook("pre-tool", event)
        journal = next(self.state.glob("*.jsonl"))
        journal.chmod(0o644)
        self.run_hook("post-tool", event)
        self.assertEqual(journal.stat().st_mode & 0o777, 0o600)

    def test_installer_preserves_existing_hooks_and_deduplicates(self) -> None:
        codex_home = self.root / "codex-home"
        codex_home.mkdir()
        config = {"hooks": {"SessionStart": [{"matcher": "x", "hooks": [{"command": "keep-me"}]}]}}
        (codex_home / "hooks.json").write_text(json.dumps(config))
        for _ in range(2):
            subprocess.run(
                [sys.executable, str(INSTALLER), "install", "--codex-home", str(codex_home)],
                capture_output=True,
                text=True,
                check=True,
            )
        installed = json.loads((codex_home / "hooks.json").read_text())
        self.assertEqual(installed["hooks"]["SessionStart"][0]["hooks"][0]["command"], "keep-me")
        commands = json.dumps(installed).count("synapse-checkpoint.py")
        self.assertEqual(commands, 4)


if __name__ == "__main__":
    unittest.main()
