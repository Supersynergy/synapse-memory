# CatBoost Search-Mode Routing

## Goal

Train a classifier to route queries to `Lex` (BM25) vs `Hybrid` (BM25+Vec RRF), minimizing latency while preserving recall.

## Training Data

20 labeled queries. Features: `query_len` (word count), `stopword_ratio`. Labels: 1=Lex, 0=Hybrid.

Rule: short keyword queries (<3 words, low stopword ratio) → Lex is faster and sufficient.
Long natural-language queries → Hybrid adds semantic lift.

## Model

```
CatBoostClassifier(iterations=50, depth=3, lr=0.3)
Model saved: ~/.claude/models/synapse_route.cbm
```

## Results

| Metric | Value |
|--------|-------|
| Training accuracy | 95% (19/20) |
| Baseline (always Lex) | 60% |
| Always Lex avg latency | 0.25ms |
| Always Hybrid avg latency | 11.53ms |
| Routed avg latency | 4.16ms |
| Speedup vs always-Hybrid | 2.8x |

## Routing Rule (learned)

- `query_len <= 2` AND `stopword_ratio < 0.2` → Lex (0.2–0.5ms)
- Otherwise → Hybrid (8–16ms, semantic coverage)

## Application

```python
from catboost import CatBoostClassifier
model = CatBoostClassifier()
model.load_model('/Users/master/.claude/models/synapse_route.cbm')

def route(query: str) -> str:
    words = query.split()
    sw = ['the','a','is','in','at','to','how','what','and','or','with','for']
    features = [[len(words), sum(1 for w in words if w in sw) / max(len(words),1)]]
    return 'Lex' if model.predict(features)[0] == 1 else 'Hybrid'
```

## Caveat

Training set is tiny (20 samples). The rule is effectively `len <= 2 → Lex`. For production, collect 500+ real queries with latency/recall labels and retrain.
