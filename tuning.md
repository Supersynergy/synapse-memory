# Synapse — Tuning Plan (2026-05-27)

> Cross-reference: `~/SpeedTuning/` (500 use cases + Top-0.001% framework).
> Distillation-rule: every action below = €/probability/longevity-positive.

## 0. Status (brutal honest)

- **Was es ist**: 304k-doc vector + BM25 daemon, SimSIMD-kernels (S4 1-bit 71×, S3 int8 46×), `/tmp/synapse.sock` :9477, MRL/f16/Hamming.
- **Echter moat**: hybrid-search 8 ms recall + 2550 claude-chats indexiert + auto-inject hook. **Niemand außer dir hat das.**
- **Bench-realität**: 7/13 #1 (54%), 10/13 top-3 (77%) — `synapse_truth_2026_05_10.md`. Drift-check Pflicht (HD-claim "1000×" wurde später korrigiert).
- **Engpass jetzt**: caller-libs warm-load tokenizer/IDF/rerank-model **pro call** statt 1× per Prozess. Picosecond pattern liegt brach.

---

## 1. Speed targets (OnceLock → real wins)

| # | Wo | Cold | Warm | Hot path | Geschätzte Δ |
|---|---|---|---|---|---|
| 1 | **MRL embedder weights** (fastembed mmap) | 200-500 ms | 0 ns (already mmap) | per query | -200 ms first-hit |
| 2 | **SimSIMD kernel dispatch ptr-table** per CPU feature | 10 µs | 1 ns | every vec-op | -1 ms / 1000 vecs |
| 3 | **Tokenizer vocab trie** (100k entries) | 100-300 ms | ns | per query | -10 ms first-tokenize |
| 4 | **BM25 IDF table** (304k docs, ~5 MB) | 50 ms | ns | every hybrid query | -5 ms/query |
| 5 | **Rerank-model (ms-marco-MiniLM)** | 200 ms | 0 (process-singleton) | every rerank | -200 ms/cold caller |
| 6 | **HNSW index header + offsets** mmap | 50 ms | ns | every ANN | -50 ms/cold |
| 7 | **Source-trust-prior table** (`known-fact`+0.022, `file-history`-0.x) | 5 ms parse | ns | every result-score | -5 ms/query |
| 8 | **Stop-word sets EN+DE** | 1 ms | ns | every BM25 | -1 ms |
| 9 | **Synapse-socket TOML config** | 2 ms | ns | every reload | -2 ms |
| 10 | **Doc-id → source-type map** (hook filter) | 20 ms | ns | every auto-inject | -20 ms/prompt |

**Σ first-hit savings**: ~500 ms cold-eliminated.
**Σ steady-state**: ~30 ms/query × 1000 queries/day ≈ 30 s/day reclaimed.

### Implementation pattern

```rust
use std::sync::LazyLock;
static IDF: LazyLock<HashMap<String, f32>> = LazyLock::new(|| {
    let path = synapse_dir().join("bm25_idf.bin");
    bincode::deserialize(&std::fs::read(path).unwrap()).unwrap()
});
pub fn idf(term: &str) -> f32 { *IDF.get(term).unwrap_or(&1.0) }
```

---

## 2. Top 0.001% applied to Synapse

### Distribution > product (#1)
Synapse hat **kein** distribution-system. 304k docs only nutzt **du**. Aktionen:
- **Public-ship**: `synapse-cli` als crates.io publish + Reddit /r/rust + HN
- **DACH-niche angle**: "Local-first vector-DB für DSGVO-Steuerberater" — keine cloud, kein BAFA-audit-aufwand
- **Demo-Repo öffentlich**: `synapse-demo` mit 1-Klick `cargo run` auf 10k Wikipedia-docs

### Specific knowledge × leverage (#2)
SimSIMD-tune-Erfahrung × Rust × German-NLP-tokenizer = unique. **Niemand DACH-seitig** verbindet das. Schreib 1 thread/week über die spezifische M4 Max optimization → DE-tech-twitter authority by default.

### Sell before build (#4)
- Vor weiterem speed-tuning: **3 telefonate** mit potentiellen kunden (Steuerberater-IT-leiter, Klinik-IT, Spedition-IT) zu "would you pay €500/mo for local-first semantic search?". 5 deep-interviews > 5000 LOC.
- Wenn ja → 1-page landing + Stripe-checkout VOR weiterem code.

### Unsexy niche × modern tool = goldader (#5)
**Top 3 verticals** für Synapse als productized:
1. **Steuerberater-mandanten-portal** (DSGVO-konform, keine cloud) → DSGVO-shield + Synapse-search = €1.2k/mo per kanzlei
2. **Krankenhaus-pathologie/radiologie reports** suchbar → €5-15k/mo
3. **Anwaltskanzlei vertrags-DB** lokal → €2-5k/mo

### Cash flow (#7)
- **Annual prepay -20%** anbieten
- **Deposit upfront** (€2k setup + monthly)
- Net-7 statt net-30

---

## 3. Via negativa (was killen)

- ❌ **Mehr engines benchen** wenn `synapse_truth_2026_05_10.md` schon gewinnt. Diminishing returns.
- ❌ **Kuzu-port** — Apple-killed Oktober 2025. Tot.
- ❌ **CUDA-pfad** — M4 Max ist Metal. Konzentrier dich auf das was du hast.
- ❌ **Multi-tenant cloud-version** — geht gegen den "local-first" moat. NUR wenn explizit verkauft.
- ❌ **Eigenes UI** — Synapse ist library/daemon. UI baut der kunde (oder du in einem separaten projekt).
- ❌ **Bench-overclaim** — KNOWN-ISSUES Day-13 audit hat 1000× CTAS claim später falsifiziert. Number-claim discipline strictly.

---

## 4. Asymmetric bet (limited downside × unbounded upside)

**Wager**: 4-wöchen-sprint "Synapse-DSGVO-Pack" — Steuerberater pilot + 1 case-study.
- **Max-loss**: 4 wochen zeit (€0 cash, du hast schon alles)
- **Floor (worst-case)**: 1 case-study + DSGVO-experten-zertifizierung lernen
- **Ceiling (best-case)**: erste €5-10k MRR + replicable template + 3 weitere kanzleien direct
- **EV**: hochgradig positiv. Mach es VOR weiterem speed-tuning.

---

## 5. Ship-this-week (5 actions, max 1 woche)

| Tag | Action | Output | Compounding |
|---|---|---|---|
| Mo | 5× cold-emails an DACH-steuerberater-IT, "kann ich 30min interview?" | 2 deep-interviews booked | sales-pipeline aufgebaut |
| Di | OnceLock-patch für BM25 IDF + tokenizer (#3+#4 oben) | -15 ms/query | speed-credibility für demo |
| Mi | landing-page `synapse-dsgvo.de` mit Stripe-checkout (€2k setup + €1.2k/mo, pre-order) | 1 LP live | sell-before-build proof |
| Do | `cargo publish synapse-cli` + Reddit /r/rust thread "I built a 8ms hybrid search in 300 LOC" | 100-500 stars target | audience #11 |
| Fr | 2 interviews + 1 case-study-outline | qualified-lead + draft | trust-compounding #12 |

---

## 6. 90-day compounding plan

**M1 (Tag 1-30)**: Distribution-foundation
- 5 deep-interviews / week (compounding #6)
- 1 public-ship / week (#11)
- DSGVO-shield + Synapse productize package
- Erste 1-2 pilot-kanzleien zu €5k pilot-pauschale

**M2 (Tag 31-60)**: Sell-before-build
- 3-5 paying pilots (€1.5k-3k MRR target)
- Case-study dokument von #1 fertig
- Synapse-cli auf crates.io 500+ stars
- 1× pro woche public DSGVO-content (#11 + #2)

**M3 (Tag 61-90)**: Compound
- €5-10k MRR (zwischen 5-8 kunden)
- Mastermind oder DSGVO-CPA-mentor (#37 + #38)
- Workshops "DSGVO-konformes Mandanten-Portal in 2 Wochen" (€2k/seat × 20 = €40k Q3-pipeline)
- Holding-struktur planen (#34) für Q4

---

## 7. Identity reframe (#24)

Statt: "Ich baue eine vector-DB"
→ Sondern: **"Ich bin DER mensch der DACH-steuerberatern DSGVO-konformes semantic-search in 2 wochen liefert, mit lokaler-installation, ohne cloud, mit case-studies von 12 kanzleien."**

Spezifität × Beweis × Erreichbarkeit = €€€ + nachfrage > kapazität.

Alle 10 speed-targets oben sind **werkzeuge zu diesem ziel** — nicht ziel-an-sich.

---

## 8. Test-frage täglich

> "Wenn ich heute 1h hätte für Synapse, was wäre die action mit höchstem € × longevity × compounding?"

Antwort fast-nie: weitere kernel-optimierung.
Meist: **kunde anrufen, public ship + post, vertrag closen, case-study schreiben.**
