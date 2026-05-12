#!/usr/bin/env python3
"""MLX timing shim for amx_neon_mlx bench.
Prints JSON: {"wa": ms, "wb": ms, "wc": ms, "wd": ms}
All times = median of 200 iterations (warm GPU).
"""
import json, time, statistics
import mlx.core as mx

ITERS = 200

def median_ms(fn, n=ITERS):
    # warm up + pre-sync
    for _ in range(5):
        r = fn(); mx.eval(r)
    samples = []
    for _ in range(n):
        t0 = time.perf_counter()
        r = fn()
        mx.eval(r)  # force Metal sync on the actual result
        samples.append((time.perf_counter() - t0) * 1000)
    return statistics.median(samples)

# W-A: 1024×512 matmul f32
A = mx.random.normal((1024, 512))
B = mx.random.normal((512, 1024))
mx.eval(A, B)
wa = median_ms(lambda: mx.matmul(A, B))

# W-B: pearson corr-matrix 220×220 (input 220×60 data matrix)
X = mx.random.normal((220, 60))
mx.eval(X)
def pearson():
    mu = X.mean(axis=1, keepdims=True)
    xc = X - mu
    norm = mx.sqrt((xc * xc).sum(axis=1, keepdims=True))
    xn = xc / (norm + 1e-8)
    return mx.matmul(xn, xn.T)
wb = median_ms(pearson)

# W-C: cosine similarity 100k×768 (128-query subset)
Q = mx.random.normal((100_000, 768))
mx.eval(Q)
def cosine_batch():
    norms = mx.sqrt((Q * Q).sum(axis=1, keepdims=True))
    qn = Q / (norms + 1e-8)
    return mx.matmul(qn[:128], qn.T)
wc = median_ms(cosine_batch)

# W-D: rolling mean window=64 over 1M f32 — cumsum trick
V = mx.random.normal((1_000_000,))
mx.eval(V)
W = 64
def rolling_mean():
    cs = mx.cumsum(V)
    return (cs[W:] - cs[:-W]) / W
wd = median_ms(rolling_mean)

print(json.dumps({"wa": wa, "wb": wb, "wc": wc, "wd": wd}))
