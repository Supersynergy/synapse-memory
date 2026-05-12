"""Integration test for synapse_py.Synapse high-level API."""
import pytest

synapse_py = pytest.importorskip("synapse_py")
Synapse = synapse_py.Synapse


def test_put_and_search(tmp_path):
    s = Synapse(path=str(tmp_path / "brain.db"))
    row_id = s.put("doc-1", "the quick brown fox jumps over the lazy dog")
    assert isinstance(row_id, int) and row_id > 0
    hits = s.search("fox", k=5)
    assert len(hits) >= 1
    assert any("fox" in text for _, text, _ in hits)
    s.close()


def test_put_with_metadata(tmp_path):
    s = Synapse(path=str(tmp_path / "brain.db"))
    s.put("doc-2", "machine learning embeddings", metadata={"source": "test"})
    hits = s.search("embeddings", k=5)
    assert any("embeddings" in text for _, text, _ in hits)
    s.close()


def test_close_raises_on_reuse(tmp_path):
    s = Synapse(path=str(tmp_path / "brain.db"))
    s.close()
    with pytest.raises(RuntimeError):
        s.search("anything")


def test_version_present():
    assert hasattr(synapse_py, "__version__")
    assert synapse_py.__version__ == "0.1.0"
