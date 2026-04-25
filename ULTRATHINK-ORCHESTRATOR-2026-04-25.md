# Ultrathink Orchestrator — Synapse Gamechanger

Date: 2026-04-25
Layer: META — coordinates 6 Agent Teams + superml feedback loops + parallel subagent spawning.
Position: above THINK-OS (which defines teams), defines DECISION RULES that pick what to spawn next.

---

## I. Orchestration Principles

1. **Parallel by default.** Independent work fans out via Agent tool with `run_in_background=true`. Teams don't synchronize unless their outputs depend.
2. **Phases are nested fractals.** Each Phase has 4-5 subagents. Each subagent can spawn its own micro-team for sub-tasks. Hierarchical, not flat.
3. **Ship daily, not weekly.** Every team must merge at least 1 PR/day. Speed of feedback compounds learning.
4. **A/B is the merge gate.** No feature lands without a measurable win in shadow mode. Bench dashboard is the source of truth.
5. **The orchestrator IS a subagent.** It runs in foreground (this conversation). Polls state, picks next action, never busy-loops.

---

## II. Decision Matrix — Spawn What When?

When idle (no active work in foreground), pick from this priority queue:

| Priority | Trigger | Action |
|---|---|---|
| P0 | Phase 1 build broken | implementer agent → fix |
| P0 | Daemon not responsive | researcher → diagnose |
| P1 | Bench regression >5% | reviewer → bisect |
| P1 | Subagent finished, output not committed | aggregator → merge + commit |
| P2 | New phase ready (deps met) | researcher → design doc |
| P2 | Design doc landed | implementer → first PR |
| P3 | Stable progress | bench-tracker → publish numbers |
| P4 | Idle | doc-sync → README update |

The orchestrator runs through this queue every loop iteration, picks highest matched, fires once, returns.

---

## III. Active Subagent Roster (this session)

| ID | Type | Mission | Status |
|---|---|---|---|
| ad96b468... | implementer | Refactor synapse-mysql to lib + rewire async crate | RUNNING |
| a8b48888... | researcher | SimSIMD coverage map (Phase 3) | RUNNING |
| a07cda55... | researcher | MLX Metal embedder design (Phase 5) | RUNNING |
| ac53eca4... | researcher | Live bench dashboard architecture (Phase 6) | RUNNING |
| ac3fd212... | researcher | Adaptive router (Phase 2B) | DONE → PHASE-2B doc |
| a181e241... | researcher | libSQL migration (Phase 2) | DONE → PHASE-2 doc |
| ad08cb8f... | researcher | WP-Bench harness (Phase 4) | DONE → PHASE-4 doc |

**4 active researchers + 1 active implementer = 5 parallel subagents.**

---

## IV. Aggregation Strategy

When subagents complete:
1. Notification arrives async → I read summary in result tag
2. If output is design doc (markdown): write it to repo at `PHASE-<N>-<TOPIC>.md`
3. If output is code: verify compile + smoke test before merge
4. If output is bench result: append to `bench/results/YYYY-MM-DD.jsonl`
5. Commit each merge with attribution: `Co-authored-by: subagent <id>`

When 2+ teams produce conflicting designs:
1. Devil's advocate skill (meta-devils-advocate-skill-killer) reviews both
2. Higher-stakes decision = bring it to user, not auto-merge
3. Otherwise: ship the simpler design, log decision in `~/.synapse/decisions.jsonl`

---

## V. Self-Improving Feedback Loops

### Loop A — Daily bench feedback
```
Nightly cron:
  bench-tracker → JSONL → DuckDB query log → CatBoost router retrain → A/B shadow
  → if shadow win >68%, promote → next day's queries faster → loop
```

### Loop B — Subagent quality feedback
```
Every subagent output:
  → score by orchestrator (was it useful? did it land code?)
  → log in `~/.synapse/agent-quality.jsonl`
  → next time pick subagent, prefer high-scorers (Thompson bandit)
```

### Loop C — Phase priority feedback
```
Every commit landed:
  → measure metric delta (latency, OPS, recall, MRR)
  → log in `~/.synapse/phase-impact.jsonl`
  → next phase priority = argmax(expected_impact / effort)
```

These three loops compound. After 30 days, system picks next-best-action faster than human planning.

---

## VI. superml Integration in Orchestrator

Orchestrator delegates 4 decisions to superml:

| Decision | Algo | Features | Outcome |
|---|---|---|---|
| "Which subagent type?" | TabPFN v2 (small) | task description embedding | implementer/researcher/reviewer/general |
| "Spawn now or batch?" | LightGBM | queue depth, hour, dependency graph | spawn vs wait |
| "Merge or hold?" | EBM (interpretable) | bench delta, test coverage, risk | auto-merge/review/reject |
| "Promote shadow?" | NGBoost (uncertainty) | A/B p50 delta + confidence | promote/extend/abort |

All 4 trained on local `~/.synapse/orchestrator-logs.duckdb`. Cold-start uses heuristics.

---

## VII. thinkrich Revenue Lens

Every orchestrator decision asks: **"Does this move €4k MRR closer?"**

Highest-impact paths to MRR:
1. WP plugin polish (Phase 4) — direct conversions
2. Live bench dashboard (Phase 6) — credibility = leads
3. AI built-ins (Phase 5) — Algolia/Pinecone replacement story
4. Synapse Cloud managed tier (Phase 7) — recurring revenue
5. Phase 1+2+3 raw speed — supports above

Tasks that don't ladder up to one of these get parked.

---

## VIII. Devils-Advocate Hook (skill-killer)

Before each Phase commits:
1. Spawn meta-devils-advocate-skill-killer agent
2. Prompt: "Find 5 reasons Phase N will fail. Be brutal."
3. If 3+ failure modes are real, pause and revise design
4. Log critique in `~/.synapse/red-team/phase-N.md`

This is the opposite of Yes-and. Forces self-honesty before public commit.

---

## IX. Master-Check Hook

Daily 09:00:
- Skill `master-check` runs all 6 verification subsystems in parallel
- Aggregates: health/wiring/checkpoint/audit/dsgvo/skills
- Output: single dashboard `~/.synapse/master-check-YYYY-MM-DD.md`
- Alert if RED on any axis → orchestrator promotes the fix to P0

This is the daily heartbeat. If master-check is GREEN for 7 consecutive days, system is safe to publish bench numbers (Phase 6).

---

## X. Concrete Today (2026-04-25 evening)

**Active state:**
- 5 subagents running in parallel
- Daemon up: synapsed PID 62724 (137k+ docs)
- Phase 1 MVP shipped + validated
- 3 design docs landed (Phase 2, 2B, 4)
- 1 implementer working on Phase 1.5 (rewrite reuse)

**When subagents complete (next 10-30 min):**
1. PHASE-3-SIMSIMD-COVERAGE.md committed
2. PHASE-5-MLX-METAL-EMBEDDER.md committed
3. PHASE-6-LIVE-BENCH-DASHBOARD.md committed
4. Phase 1.5 lib refactor merged + bench rerun

**Then orchestrator picks next P2:**
- Spawn implementer for PHASE-3-SIMSIMD top-3 kernels
- OR spawn implementer for PHASE-2-LIBSQL backend trait scaffolding
- Decision driven by superml routing once router has logs

**This week DoD:**
- All 6 PHASE-*.md docs landed ✅ in flight
- Phase 1.5 binary reusing rewrite + ACL ✅ in flight
- Phase 1 sysbench validated via opensrv v0.10 fork (Phase 1.5+ task)
- Live bench dashboard scaffold (Astro init)

---

## XI. State Persistence

`~/.synapse/teams-state.json`:
```json
{
  "session_id": "2026-04-25-evening",
  "active_teams": ["alpha+", "epsilon", "zeta", "omega"],
  "completed_designs": ["phase-2", "phase-2b", "phase-4"],
  "pending_designs": ["phase-3", "phase-5", "phase-6"],
  "ship_today": ["a00850e Phase 1 MVP", "b96fcfb design docs trio"],
  "blockers": ["opensrv-mysql 0.7 lacks TLS+caching_sha2 (sysbench bench)"],
  "next_p0": "merge Phase 1.5 lib refactor when implementer returns",
  "mrr_eta": "phase 4 Day 49"
}
```

Updated atomically by each subagent completion.

---

## XII. Closing — The "Ultrathink" Frame

Linear plan: do A, then B, then C. 90 days.
Ultrathink plan: A+B+C+D+E+F all parallel. Daily merges. 6 teams compound. ML routes traffic. Devils-advocate kills bad bets. Master-check guards regressions. Bench dashboard publishes weekly.

**Result not 90 days but 30 days to gamechanger.**

The system gets smarter every night.
The orchestrator runs in foreground, fires subagents, aggregates outputs, commits, picks next, repeats.
We are building a system that builds itself.
