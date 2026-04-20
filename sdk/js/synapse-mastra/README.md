# @synapse/mastra

Mastra `MastraMemory` adapter backed by [synapse](https://github.com/Supersynergy/synapse).

Interface spec: https://github.com/mastraai/mastra (23k★)

## Install

```bash
npm install @synapse/mastra
```

## Usage

```ts
import { SynapseMemory } from "@synapse/mastra";

const mem = new SynapseMemory("/tmp/synapse.sock");

await mem.remember({ id: "ctx:1", content: "User prefers TypeScript." });
const hits = await mem.recall("language preference", { limit: 5 });
await mem.forget("ctx:1");
```

## API

| Method | Signature | Description |
|--------|-----------|-------------|
| `remember` | `(entry: MastraMemoryEntry) => Promise<number>` | Store memory, returns doc id |
| `recall` | `(query, opts?) => Promise<MastraMemoryEntry[]>` | Hybrid search over memories |
| `forget` | `(id: string) => Promise<void>` | Soft-delete by id |

## Prerequisites

synapsed running on unix socket:
```bash
synapse serve --socket /tmp/synapse.sock
```
