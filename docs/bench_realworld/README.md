# Real-World Benchmark Suite — 50 Usecases

Goal: translate Synapse's technical speed-up (53×–71× vs scalar) into
human-impact stories that a non-engineer cares about. Every entry here is a
**runnable harness** that an end-user could point at their own data.

## Index

Legend: `corpus size · query shape · metric · consumer target`

### A. Personal Knowledge (`a01`–`a10`)

| id | scenario | harness | status |
|----|----------|---------|--------|
| a01 | Obsidian vault semantic search         | `01_obsidian.py` | ✅ shipped |
| a02 | Notion export recall                   | `harness.py --src notion_export/` | template |
| a03 | Apple Notes SQLite                     | `harness.py --src Notes.db` | template |
| a04 | Logseq daily journal                   | `harness.py --src logseq/` | template |
| a05 | Roam backlink auto-suggest             | `harness.py --src roam.json` | template |
| a06 | Kindle highlights `.csv`               | `harness.py --src kindle.csv` | template |
| a07 | Readwise snippet archive               | `harness.py --src readwise.ndjson` | template |
| a08 | Voice-memo Whisper transcripts         | `harness.py --src memos/` | template |
| a09 | Pocket/Instapaper bookmark archive     | `harness.py --src pocket.json` | template |
| a10 | PDF highlights (Zotero/Mendeley)       | `harness.py --src pdfs/` | template |

### B. AI-Agent Memory (`b11`–`b20`)

| id | scenario | harness | status |
|----|----------|---------|--------|
| b11 | ChatGPT history recall                 | `11_chatgpt_history.py` | ✅ shipped |
| b12 | Claude conversation memory             | `harness.py --src claude_export/` | template |
| b13 | Cursor project memory                  | `harness.py --src .cursor/chat/` | template |
| b14 | Copilot long-context stitch            | synthetic bench | template |
| b15 | Mem0 substrate replacement             | plug `mem0_adapter.py` | ✅ |
| b16 | Letta/MemGPT replacement               | same | ✅ |
| b17 | LangChain RAG pipeline                 | plug `langchain_adapter.py` | ✅ |
| b18 | Agent-swarm CRDT memory                | `harness.py --crdt 10agents` | template |
| b19 | Multi-tenant isolation                 | synthetic | template |
| b20 | Long-horizon plan recall               | synthetic | template |

### C. Email & Messaging (`c21`–`c25`)

| id | scenario | harness | status |
|----|----------|---------|--------|
| c21 | Gmail mbox search                      | `21_gmail_mbox.py` | ✅ shipped |
| c22 | Slack workspace export                 | `harness.py --src slack_export/` | template |
| c23 | Signal/iMessage chat.db                | `harness.py --src chat.db` | template |
| c24 | WhatsApp `.txt` export                 | `harness.py --src wa.txt` | template |
| c25 | Outlook `.pst` archive                 | external converter + template | template |

### D. Code & Dev (`d26`–`d32`)

| id | scenario | status |
|----|----------|--------|
| d26 | Monorepo symbol search | template |
| d27 | Linux kernel semantic grep | template |
| d28 | Git-log explanation fetch | template |
| d29 | Stack-Overflow offline KB | template |
| d30 | Rust crate docstring nav | template |
| d31 | Jupyter-notebook archive | template |
| d32 | CI-log triage | template |

### E. Media (`e33`–`e38`)

| id | scenario | status |
|----|----------|--------|
| e33 | Photo library CLIP search | template |
| e34 | Apple Music + lyrics taste | template |
| e35 | Podcast-episode jump | template |
| e36 | YouTube watch-history | template |
| e37 | Instagram save-archive | template |
| e38 | TikTok bookmark store | template |

### F. Work & Business (`f39`–`f44`)

| id | scenario | status |
|----|----------|--------|
| f39 | Zendesk similar-ticket | template |
| f40 | Linear/Jira dup-detect | template |
| f41 | Salesforce contact-notes | template |
| f42 | Confluence onboarding-Q | template |
| f43 | CS-chat auto-reply | template |
| f44 | Legal-contract clause match | template |

### G. Research (`g45`–`g48`)

| id | scenario | status |
|----|----------|--------|
| g45 | arXiv PDF mirror | template |
| g46 | Semantic-Scholar abstracts | template |
| g47 | Clinical-notes EHR (synthetic) | template |
| g48 | Patent prior-art | template |

### H. Health & Life (`h49`–`h50`)

| id | scenario | status |
|----|----------|--------|
| h49 | Apple-Health + journal fuse | template |
| h50 | Recipe/meal-plan assistant | template |

## Shared metrics (all harnesses emit)

| metric | unit | non-geek framing |
|---|---|---|
| p50 latency | µs | "how fast on average" |
| p95 latency | µs | "how fast for 95 % of queries" |
| p99 latency | µs | "worst-case, 1-in-100" |
| recall@10 | 0..1 | "how often the right answer is in top 10" |
| corpus build time | s | "one-time setup time" |
| RAM footprint | MB | "how much memory it eats" |
| battery delta | mWh | "cost to your laptop's charge" |

## Copy-ready headlines (per usecase class)

- **Personal Knowledge**: "Every note you've ever written, instantly searchable"
- **Agent Memory**: "AI that actually remembers you"
- **Email**: "Inbox zero, actually"
- **Code**: "Find the right line, first try"
- **Media**: "Your photos, voice memos, music — one brain"
- **Work**: "Your customer knows. So should your team."
- **Research**: "Literature review in an afternoon"
- **Health**: "Your body's second brain"

## How to run

```bash
cd crates/synapse-py
maturin develop --release --features simsimd
cd ../../bench/realworld
python harness.py --help
python 01_obsidian.py ~/Vaults/MyNotes --queries queries.txt
```
