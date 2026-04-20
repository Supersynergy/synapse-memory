# @synapse/vercel-ai

Vercel AI SDK memory provider backed by [synapse](https://github.com/Supersynergy/synapse).

Interface spec: https://sdk.vercel.ai/docs/reference/ai-sdk-core/generate-text

## Install

```bash
npm install @synapse/vercel-ai
```

## Usage

```ts
import { createMemoryProvider } from "@synapse/vercel-ai";
import { generateText } from "ai";

const memory = createMemoryProvider({ sockPath: "/tmp/synapse.sock" });

// In your API route:
const context = await memory.retrieve(sessionId, userMessage);
const result = await generateText({ model, messages: [...context, { role: "user", content: userMessage }] });
await memory.store(sessionId, [{ role: "user", content: userMessage }, { role: "assistant", content: result.text }]);
```

## API

| Function | Description |
|----------|-------------|
| `createMemoryProvider(opts)` | Returns `MemoryProvider` |
| `provider.store(sessionId, messages)` | Persist messages |
| `provider.retrieve(sessionId, query, limit?)` | Retrieve relevant context |
