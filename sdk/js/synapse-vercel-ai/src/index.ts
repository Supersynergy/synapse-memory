/**
 * @synapse/vercel-ai — Vercel AI SDK memory provider
 * Interface spec: https://sdk.vercel.ai/docs/reference/ai-sdk-core/generate-text
 * Hooks into generateText `messages` persistence and useChat via middleware pattern.
 */
import { Synapse, type Hit } from "@synapse/sdk";

export interface Message {
  role: "user" | "assistant" | "system";
  content: string;
}

export interface MemoryProvider {
  store(sessionId: string, messages: Message[]): Promise<void>;
  retrieve(sessionId: string, query: string, limit?: number): Promise<Message[]>;
}

export interface SynapseMemoryProviderOptions {
  sockPath?: string;
  /** Prefix stored docs with sessionId for isolation */
  namespace?: string;
}

/**
 * createMemoryProvider — returns a MemoryProvider that persists messages to synapse.
 * Usage: wrap generateText calls with store()/retrieve() for long-term context.
 */
export function createMemoryProvider(opts: SynapseMemoryProviderOptions = {}): MemoryProvider {
  const client = new Synapse(opts.sockPath ?? "/tmp/synapse.sock");
  const ns = opts.namespace ?? "vai";

  return {
    async store(sessionId: string, messages: Message[]): Promise<void> {
      const docs = messages.map((m) => ({
        text: m.content,
        uri: `${ns}:${sessionId}:${m.role}:${Date.now()}`,
        meta: { role: m.role, sessionId },
        embed: true,
      }));
      await client.putBatch(docs);
    },

    async retrieve(sessionId: string, query: string, limit = 8): Promise<Message[]> {
      const hits: Hit[] = await client.search(`${sessionId} ${query}`, {
        mode: "Hybrid",
        limit,
        embedQuery: true,
      });
      return hits
        .filter((h) => h.uri?.startsWith(`${ns}:${sessionId}:`))
        .map((h) => ({
          role: ((h as any).meta?.role ?? "assistant") as Message["role"],
          content: h.text,
        }));
    },
  };
}

export { Synapse };
