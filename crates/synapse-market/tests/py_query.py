"""
Cross-language ABI stub for synapse-market.
Status: NOT EXECUTABLE — requires pyo3 wheel (feature = "python").

Once built (`maturin develop --features python`):
    import synapse_market
    s = synapse_market.Series.open("abi.smx")
    bars = s.range(ts_start, ts_end)
    # Returns list of dicts: [{ts, open, high, low, close, volume}, ...]
    import json, hashlib
    rows = sorted(bars, key=lambda b: b["ts"])
    j = json.dumps(rows, sort_keys=True, separators=(",", ":"))
    blake3_hash = ...  # pip install blake3
    print(blake3_hash.of(j.encode()).hex())

The abi_cross_lang.rs test will compare this hash against the Rust reference hash.
"""
raise NotImplementedError("pyo3 binding not yet built — add feature 'python' to synapse-market")
