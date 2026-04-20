/**
 * Mastra quickstart — synapse memory backend
 * Run: npx tsx examples/mastra-quickstart.ts
 * Requires synapsed running: synapse serve --socket /tmp/synapse.sock
 */
import { SynapseMemory } from "../src/index.js";

const mem = new SynapseMemory();

// Store some facts
await mem.remember({ id: "fact:1", content: "The Eiffel Tower is 330m tall.", metadata: { source: "wiki" } });
await mem.remember({ id: "fact:2", content: "Paris is the capital of France.", metadata: { source: "wiki" } });
await mem.remember({ id: "fact:3", content: "Berlin is the capital of Germany.", metadata: { source: "wiki" } });

// Recall relevant entries
const results = await mem.recall("capital of France", { limit: 3, mode: "Hybrid" });
console.log("Recall results:", JSON.stringify(results, null, 2));

// Forget a specific memory
await mem.forget("fact:1");
console.log("Forgot fact:1");

mem.close();
