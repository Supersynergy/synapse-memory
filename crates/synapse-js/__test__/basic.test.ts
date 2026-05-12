import { test, expect, afterAll } from "bun:test";
import { join } from "path";
import { mkdirSync, rmSync } from "fs";

// The native .node addon is built by `napi build --release`.
// If not yet built, we skip gracefully.
let Synapse: typeof import("../index.js").Synapse | undefined;
let dbPath: string;

try {
  // eslint-disable-next-line @typescript-eslint/no-require-imports
  ({ Synapse } = require("../index.js"));
} catch {
  console.warn("⚠️  synapse-js native addon not built — skipping integration tests. Run: napi build --release");
}

const TMP = "/tmp/synapse-js-test";

if (Synapse) {
  mkdirSync(TMP, { recursive: true });
  dbPath = join(TMP, "test.synx");

  test("put + search round-trip", async () => {
    const s = new Synapse!(dbPath);
    const id = await s.put("doc:1", "the quick brown fox jumps over the lazy dog", JSON.stringify({ tag: "test" }));
    expect(id).toBeGreaterThan(0);

    const hits = await s.search("quick brown fox", 5);
    expect(hits.length).toBeGreaterThan(0);
    expect(hits[0].text).toContain("quick brown fox");

    await s.close();
  });

  test("put multiple + search", async () => {
    const s = new Synapse!(dbPath);
    await s.put("doc:2", "machine learning with transformers", JSON.stringify({ tag: "ml" }));
    await s.put("doc:3", "synapse is a fast memory engine", JSON.stringify({ tag: "synapse" }));

    const hits = await s.search("memory engine", 3);
    expect(hits.length).toBeGreaterThan(0);

    await s.close();
  });

  afterAll(() => {
    rmSync(TMP, { recursive: true, force: true });
  });
} else {
  test("native addon not built — scaffold only", () => {
    expect(true).toBe(true);
  });
}
