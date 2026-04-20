/**
 * @synapse/mastra — Mastra MastraMemory adapter
 * Interface spec: https://github.com/mastraai/mastra/blob/main/packages/core/src/memory/types.ts
 */
import { Synapse, type PutRequest, type Hit } from "@synapse/sdk";

export interface MastraMemoryEntry {
  id?: string;
  content: string;
  metadata?: Record<string, unknown>;
}

export interface MastraMemorySearchOptions {
  limit?: number;
  mode?: "Lex" | "Vec" | "Hybrid";
}

/** Implements the Mastra `MastraMemory` interface backed by synapsed. */
export class SynapseMemory {
  private client: Synapse;

  constructor(sockPath = "/tmp/synapse.sock") {
    this.client = new Synapse(sockPath);
  }

  /** Store a memory entry. Returns the synapse doc id. */
  async remember(entry: MastraMemoryEntry): Promise<number> {
    const req: PutRequest = {
      text: entry.content,
      uri: entry.id,
      meta: entry.metadata,
      embed: true,
    };
    return this.client.put(req);
  }

  /** Retrieve relevant memories by semantic/lexical search. */
  async recall(query: string, opts: MastraMemorySearchOptions = {}): Promise<MastraMemoryEntry[]> {
    const hits: Hit[] = await this.client.search(query, {
      mode: opts.mode ?? "Hybrid",
      limit: opts.limit ?? 10,
      embedQuery: opts.mode === "Vec" || opts.mode === "Hybrid",
    });
    return hits.map((h) => ({
      id: h.uri ?? String(h.id),
      content: h.text,
      metadata: { score: h.score, title: h.title },
    }));
  }

  /** Remove a memory by storing a tombstone with empty text (synapse has no delete RPC yet). */
  async forget(id: string): Promise<void> {
    await this.client.put({ text: "", uri: `__deleted__:${id}`, embed: false });
  }

  close(): void {
    this.client.close();
  }
}

export { Synapse };
