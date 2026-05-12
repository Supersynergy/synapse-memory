/**
 * synapse-market Bun:FFI wrapper
 * Build dylib first: cargo build -p synapse-market-ts --release
 */
import { dlopen, FFIType, ptr, read, suffix } from "bun:ffi";
import { resolve } from "path";

const LIB_PATH =
  process.env.SMX_LIB_PATH ??
  resolve(
    import.meta.dir,
    `../../../target/release/libsynapse_market_ffi.${suffix}`
  );

const {
  symbols: { smx_market_open, smx_market_close, smx_ingest_ohlcv, smx_series_range_close },
} = dlopen(LIB_PATH, {
  smx_market_open: {
    args: [FFIType.cstring],
    returns: FFIType.ptr,
  },
  smx_market_close: {
    args: [FFIType.ptr],
    returns: FFIType.void,
  },
  smx_ingest_ohlcv: {
    args: [FFIType.ptr, FFIType.cstring, FFIType.ptr, FFIType.usize],
    returns: FFIType.i32,
  },
  smx_series_range_close: {
    args: [FFIType.ptr, FFIType.cstring, FFIType.i64, FFIType.i64, FFIType.ptr, FFIType.usize],
    returns: FFIType.i64,
  },
});

export class Market {
  private handle: number;

  private constructor(handle: number) {
    this.handle = handle;
  }

  static open(path: string): Market {
    const h = smx_market_open(Buffer.from(path + "\0"));
    if (!h) throw new Error(`smx_market_open failed: ${path}`);
    return new Market(h as number);
  }

  close(): void {
    smx_market_close(this.handle);
  }

  series(ticker: string): Series {
    return new Series(this.handle, ticker);
  }
}

export class Series {
  constructor(
    private readonly marketHandle: number,
    private readonly ticker: string
  ) {}

  /** Ingest rows: array of [ts, open, high, low, close, volume] */
  append(rows: [number, number, number, number, number, number][]): void {
    const buf = new Float64Array(rows.length * 6);
    rows.forEach((r, i) => {
      buf.set(r, i * 6);
    });
    const res = smx_ingest_ohlcv(
      this.marketHandle,
      Buffer.from(this.ticker + "\0"),
      ptr(buf),
      rows.length
    );
    if (res !== 0) throw new Error("smx_ingest_ohlcv failed");
  }

  /** Return close prices as Float64Array for [start, end] range */
  range(start: bigint, end: bigint, maxLen = 10_000): Float64Array {
    const out = new Float64Array(maxLen);
    const n = smx_series_range_close(
      this.marketHandle,
      Buffer.from(this.ticker + "\0"),
      start,
      end,
      ptr(out),
      maxLen
    ) as bigint;
    if (n < 0n) throw new Error("smx_series_range_close failed");
    return out.slice(0, Number(n));
  }
}
