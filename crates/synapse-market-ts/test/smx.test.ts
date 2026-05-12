/**
 * Smoke-tests for synapse-market Bun:FFI bindings.
 * Run after: cargo build -p synapse-market-ts --release
 * Then: bun test (from crates/synapse-market-ts/)
 *
 * NOTE: tests are skipped if the dylib is not built yet.
 */
import { describe, it, expect, beforeAll } from "bun:test";
import { existsSync } from "fs";
import { resolve } from "path";

const DYLIB = resolve(
  import.meta.dir,
  "../../../target/release/libsynapse_market_ffi.dylib"
);
const SKIP = !existsSync(DYLIB);

describe("synapse-market FFI", () => {
  let Market: typeof import("../index.ts").Market;
  let Series: typeof import("../index.ts").Series;

  beforeAll(async () => {
    if (SKIP) return;
    const mod = await import("../index.ts");
    Market = mod.Market;
    Series = mod.Series;
  });

  it("opens an in-memory-like db", () => {
    if (SKIP) { console.log("SKIP: dylib not built"); return; }
    const m = Market.open("/tmp/smx_test_bun.db");
    expect(m).toBeDefined();
    m.close();
  });

  it("appends OHLCV rows", () => {
    if (SKIP) return;
    const m = Market.open("/tmp/smx_test_bun2.db");
    const s = m.series("AAPL");
    s.append([[1_700_000_000, 100, 105, 99, 103, 1000]]);
    m.close();
  });

  it("range returns correct closes", () => {
    if (SKIP) return;
    const m = Market.open("/tmp/smx_test_bun3.db");
    const s = m.series("MSFT");
    s.append([
      [1_700_000_000, 100, 105, 99, 103, 1000],
      [1_700_086_400, 103, 110, 102, 108, 1200],
    ]);
    const closes = s.range(1_700_000_000n, 1_700_086_400n);
    expect(closes.length).toBe(2);
    expect(closes[0]).toBeCloseTo(103.0);
    m.close();
  });

  it("range returns empty for unknown ticker", () => {
    if (SKIP) return;
    const m = Market.open("/tmp/smx_test_bun4.db");
    const closes = m.series("UNKNOWN").range(0n, 1n);
    expect(closes.length).toBe(0);
    m.close();
  });

  it("multiple tickers isolated", () => {
    if (SKIP) return;
    const m = Market.open("/tmp/smx_test_bun5.db");
    m.series("SPY").append([[1_700_000_000, 400, 405, 399, 402, 5000]]);
    m.series("QQQ").append([[1_700_000_000, 300, 310, 298, 305, 3000]]);
    const spy = m.series("SPY").range(0n, 2_000_000_000n);
    const qqq = m.series("QQQ").range(0n, 2_000_000_000n);
    expect(spy[0]).toBeCloseTo(402.0);
    expect(qqq[0]).toBeCloseTo(305.0);
    m.close();
  });
});
