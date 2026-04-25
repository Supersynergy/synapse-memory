#!/usr/bin/env node
// Ingest bench results from ~/projects/synapse/bench/results/YYYY-MM-DD/*.jsonl
// Aggregate latest-per-engine into src/data/latest.json + history.json
import { readdir, readFile, writeFile } from 'node:fs/promises';
import { join } from 'node:path';

const RESULTS_DIR = '../../bench/results';
const OUT_LATEST = '../src/data/latest.json';
const OUT_HISTORY = '../src/data/history.json';

async function main() {
  const days = await readdir(RESULTS_DIR).catch(() => []);
  days.sort().reverse(); // newest first

  const latest = {};
  const history = [];

  for (const day of days) {
    const dayDir = join(RESULTS_DIR, day);
    const files = await readdir(dayDir).catch(() => []);
    for (const f of files) {
      if (!f.endsWith('.jsonl')) continue;
      const content = await readFile(join(dayDir, f), 'utf-8');
      for (const line of content.split('\n').filter(Boolean)) {
        try {
          const row = JSON.parse(line);
          history.push({ ...row, date: day });
          const k = `${row.suite}::${row.engine}::${row.metric}`;
          if (!latest[k]) latest[k] = { ...row, date: day };
        } catch {}
      }
    }
  }

  await writeFile(OUT_LATEST, JSON.stringify(latest, null, 2));
  await writeFile(OUT_HISTORY, JSON.stringify(history, null, 2));
  console.log(`ingested ${days.length} days, ${Object.keys(latest).length} metrics, ${history.length} history rows`);
}

main();
