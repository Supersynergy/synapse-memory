# PHASE-6: Live Bench Dashboard (synapse.sh/bench)

Date: 2026-04-25 · Owner: Team Omega
Source: subagent ac53eca4027c337f4

## Goal
Public live bench dashboard. Weekly auto-updates. Drives 5k★ + HN top-10 launch + €4k MRR funnel.

## Tech Stack Pick
- **Frontend:** Astro + Chart.js mini (~50KB)
- **Hosting:** GitHub Pages + `.nojekyll`
- **CI:** GitHub Actions ARM runner (`macos-15-arm64`)
- **Domain:** Cloudflare CNAME → `synapse.sh/bench`
- **Data:** static `.jsonl` committed nightly to `bench/results/YYYY-MM-DD/`

Reasoning: zero runtime backend, fastest deploy, reproducible (committed JSON), free hosting.

## 3 Most Important Pages

### 1. Landing — top-10 vs competitors
Charts:
- Vec p50 latency (Synapse 0.28ms vs Chroma/Qdrant/sqlite-vec/LanceDB)
- BM25 throughput (Synapse vs Meilisearch — 187× kicker)
- RAM efficiency (64MB vs 512MB MySQL)
- Cold-start (0.69ms vs 5-30s)
- Single-file disk (1.3MB/1k vs 4.4MB Chroma)

### 2. /vs-mysql — close-up MySQL battle
- TPC-C tps at 1/4/8/16 conn
- sysbench oltp_point_select 1t/8t/64t
- YCSB workloads A-F
- Resource graph: RAM/CPU per OPS

### 3. /vs-pinecone & /vs-algolia — SaaS killers
- recall@k vs $/request
- cold-start vs €/mo
- Migration calculator: enter records → see savings

## Suites Covered (publish weekly)
1. real_competitors.py (1k vec)
2. bench_scale_ladder.py (1k-1M)
3. synx bench (ping/lex/vec/hybrid)
4. sysbench oltp_point_select 1t/8t/64t
5. go-ycsb workloads A-F
6. LoCoMo recall@1/5/10
7. BEIR subset (5 datasets)
8. WP-Bench 5 scenarios

## Anti-Cherry-Pick Guards
- Bench harness commits seed value
- Runs 3× takes median
- Raw stderr logged in artifact
- Reproducible build SHA printed
- All input data hashed (BLAKE3) and verified
- Reproducibility badge on landing: "git checkout <sha> && cargo bench"

## CI Cost
- GitHub Actions ARM `macos-15-arm64`: $0.16/min
- 30min nightly × 30 days = ~$144/mo
- **Alternative:** Hetzner CX52 €25/mo + self-hosted runner = 5× cheaper, recommended after 3-month proof
- Initial GHA → switch when bench bills exceed €100/mo

## Marketing Hooks
- "Live bench. We update every night. No cherry-pick. SHA pinned."
- Pin tweet template: "Synapse beat Pinecone by 645× on latency. See live: synapse.sh/bench"
- HN comment template: "Reproducible bench at synapse.sh/bench, runs every night via GitHub Actions"

## 5 Viral Charts for Landing (drive shares)
1. Vec p50 vs world (Synapse 0.28ms wins by 970×)
2. Search killer 187× (1500ms → 8ms)
3. RAM efficiency (64MB vs 512MB)
4. Cold start (0.69ms vs 5-30s)
5. Single-file deploy demo (cp file.db = backup)

## Phase 6 Timeline (Days 71-84)

| Days | Sprint |
|---|---|
| 71-74 | Spec + Astro scaffold |
| 75-77 | JSON schema + benchmark wiring |
| 78-80 | CI runner + nightly cron |
| 81-83 | Soft launch (10 design partners only) |
| 84 | Public launch — HN/Reddit/Twitter |

## DoD Phase 6
- 8 suites all running nightly
- Dashboard live at synapse.sh/bench
- 5k+ unique visitors first week
- 1 mainstream tech press citation
- HN top-10 (3+ days on front page)

## Status: Spec ready. Phase 6 kickoff Day 71.
