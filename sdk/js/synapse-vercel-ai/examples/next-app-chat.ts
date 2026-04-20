/**
 * Vercel AI SDK + synapse memory — Next.js route example
 * Drop this into: app/api/chat/route.ts
 * Requires: synapsed running at /tmp/synapse.sock
 */
import { createMemoryProvider } from "../src/index.js";

const memory = createMemoryProvider({ sockPath: "/tmp/synapse.sock" });

// Simulates what a Next.js API route handler would do
async function handleChatRequest(sessionId: string, userMessage: string) {
  // 1. Retrieve relevant past messages
  const context = await memory.retrieve(sessionId, userMessage, 5);
  console.log(`Context (${context.length} msgs):`, context.map((m) => m.content.slice(0, 40)));

  // 2. Build messages array for AI SDK generateText (mocked here)
  const messages = [
    ...context,
    { role: "user" as const, content: userMessage },
  ];

  // 3. [In real app] const result = await generateText({ model, messages });
  const assistantReply = `Echo: ${userMessage} (context: ${context.length} msgs)`;

  // 4. Persist this exchange
  await memory.store(sessionId, [
    { role: "user", content: userMessage },
    { role: "assistant", content: assistantReply },
  ]);

  return assistantReply;
}

// Demo
const session = "sess_demo_001";
await handleChatRequest(session, "What is the capital of France?");
const reply = await handleChatRequest(session, "Tell me more about it.");
console.log("Reply:", reply);
