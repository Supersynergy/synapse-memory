/**
 * CopilotKit + synapse persistent context store example
 * Run: npx tsx examples/copilotkit-store.ts
 * Requires synapsed at /tmp/synapse.sock
 */
import { SynapseCopilotStore } from "../src/index.js";

const store = new SynapseCopilotStore("/tmp/synapse.sock", "demo");

// Set user preferences
await store.setContext({ key: "user:lang", value: "TypeScript", categories: ["preferences"] });
await store.setContext({ key: "user:project", value: "Building an e-commerce platform", categories: ["project"] });
await store.setContext({ key: "user:team", value: "5 engineers, remote-first", categories: ["context"] });

// Retrieve exact key
const lang = await store.getContext("user:lang");
console.log("Language preference:", lang?.value);

// Semantic search
const results = await store.searchContext("what is the user building", 5);
console.log("Search results:", results.map((r) => `${r.key}: ${r.value}`));

store.close();
