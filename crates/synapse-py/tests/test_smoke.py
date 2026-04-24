"""End-to-end smoke tests for the `synapse` Python bindings.

Run with:
    maturin develop --release --features simsimd
    pytest tests/ -v
"""

import math
import random
import pytest

synapse = pytest.importorskip("synapse")


def _unit(v):
    n = math.sqrt(sum(x * x for x in v)) or 1.0
    return [x / n for x in v]


def test_version_attribute_present():
    assert hasattr(synapse, "__version__")


def test_truncate_row_produces_unit_norm():
    v = synapse.truncate_row([3.0, 4.0, 9.0, 9.0], 2)
    assert len(v) == 2
    norm = math.sqrt(v[0] ** 2 + v[1] ** 2)
    assert abs(norm - 1.0) < 1e-5


def test_cos_f32_identity_is_one():
    v = [1.0, 2.0, 3.0, 4.0]
    assert abs(synapse.cos_f32(v, v) - 1.0) < 1e-4


def test_hamming_b8_self_is_zero():
    q = [0xAB] * 16
    assert synapse.hamming_b8(q, [q]) == [0.0]


def test_f16_index_exact_match_and_half_ram():
    rows = [
        (1, _unit([1.0, 0.0, 0.0, 0.0])),
        (2, _unit([0.0, 1.0, 0.0, 0.0])),
    ]
    idx = synapse.F16Index.build(rows)
    hits = idx.search(_unit([1.0, 0.0, 0.0, 0.0]), k=1)
    assert hits[0][0] == 1
    # 2 rows × 4 dim × 2 bytes = 16
    assert idx.packed_bytes() == 16
    assert idx.dim() == 4


def test_i8_index_exact_match():
    rows = [
        (1, _unit([1.0, 0.0, 0.0, 0.0])),
        (2, _unit([0.0, 1.0, 0.0, 0.0])),
        (3, _unit([0.0, 0.0, 1.0, 0.0])),
    ]
    idx = synapse.I8Index.build(rows)
    hits = idx.search(_unit([1.0, 0.0, 0.0, 0.0]), k=1)
    assert hits[0][0] == 1


def test_hamming_index_and_rerank_pipeline():
    random.seed(0)
    rows = []
    for i in range(200):
        rows.append((i, _unit([random.gauss(0.0, 1.0) for _ in range(32)])))
    h = synapse.HammingIndex.build(rows)
    i8 = synapse.I8Index.build(rows)
    q = _unit([random.gauss(0.0, 1.0) for _ in range(32)])
    hits = synapse.rerank(h, i8, q, k=5, candidates=30)
    assert len(hits) == 5
    # scores must be descending
    scores = [s for _, s in hits]
    assert scores == sorted(scores, reverse=True)


def test_adaptive_router_learns():
    r = synapse.AdaptiveRouter()
    # High-corpus, no-budget, full-recall → should pick int8-family.
    s = r.choose(100_000, 0, 0.0)
    assert s in {"rayon", "simsimd_f32", "simsimd_i8"}
    # Feed 50 good observations of int8 → posterior moves up.
    for _ in range(50):
        r.observe("simsimd_i8", 280.0, 0.98)
    post = dict((name, (recall, us)) for name, recall, us in r.posterior())
    assert post["simsimd_i8"][0] > 0.9


def test_brain_put_and_lex_search(tmp_path):
    path = str(tmp_path / "brain.db")
    b = synapse.Brain(path)
    b.put_text("the quick brown fox", uri="doc://fox")
    b.put_text("lorem ipsum dolor sit amet")
    hits = b.search_lex("fox", limit=5)
    assert any("fox" in h[1] for h in hits)
