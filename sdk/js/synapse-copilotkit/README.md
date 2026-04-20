# @synapse/copilotkit

CopilotKit persistent context store backed by [synapse](https://github.com/Supersynergy/synapse).

Interface spec: https://github.com/CopilotKit/CopilotKit (30k★)

## Install

```bash
npm install @synapse/copilotkit
```

## Usage

```ts
import { SynapseCopilotStore } from "@synapse/copilotkit";

const store = new SynapseCopilotStore("/tmp/synapse.sock");

await store.setContext({ key: "user:pref", value: "Prefers concise answers", categories: ["preferences"] });
const results = await store.searchContext("user preferences", 5);
```

## API

| Method | Description |
|--------|-------------|
| `setContext(entry)` | Upsert a context entry |
| `getContext(key)` | Exact-key lookup |
| `searchContext(query, limit?)` | Hybrid semantic search |
| `clearContext(key)` | Soft-delete entry |
