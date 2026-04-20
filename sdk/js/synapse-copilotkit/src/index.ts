/**
 * @synapse/copilotkit — CopilotKit persistent context store
 * Interface spec: https://github.com/CopilotKit/CopilotKit (30k★)
 * Implements CopilotKitStore interface for persistent memory across sessions.
 */
import { Synapse, type Hit } from "@synapse/sdk";

export interface CopilotContextEntry {
  key: string;
  value: string;
  categories?: string[];
}

/** CopilotKitStore interface — provides get/set/search over persistent context */
export interface CopilotKitStore {
  setContext(entry: CopilotContextEntry): Promise<void>;
  getContext(key: string): Promise<CopilotContextEntry | null>;
  searchContext(query: string, limit?: number): Promise<CopilotContextEntry[]>;
  clearContext(key: string): Promise<void>;
}

export class SynapseCopilotStore implements CopilotKitStore {
  private client: Synapse;
  private prefix: string;

  constructor(sockPath = "/tmp/synapse.sock", namespace = "cpk") {
    this.client = new Synapse(sockPath);
    this.prefix = namespace;
  }

  async setContext(entry: CopilotContextEntry): Promise<void> {
    await this.client.put({
      text: entry.value,
      uri: `${this.prefix}:${entry.key}`,
      title: entry.key,
      meta: { categories: entry.categories ?? [] },
      embed: true,
    });
  }

  async getContext(key: string): Promise<CopilotContextEntry | null> {
    const hits: Hit[] = await this.client.search(key, {
      mode: "Lex",
      limit: 1,
    });
    const hit = hits.find((h) => h.uri === `${this.prefix}:${key}`);
    if (!hit) return null;
    return { key, value: hit.text };
  }

  async searchContext(query: string, limit = 10): Promise<CopilotContextEntry[]> {
    const hits: Hit[] = await this.client.search(query, {
      mode: "Hybrid",
      limit,
      embedQuery: true,
    });
    return hits
      .filter((h) => h.uri?.startsWith(`${this.prefix}:`))
      .map((h) => ({
        key: h.title ?? h.uri?.replace(`${this.prefix}:`, "") ?? String(h.id),
        value: h.text,
      }));
  }

  async clearContext(key: string): Promise<void> {
    await this.client.put({
      text: "",
      uri: `${this.prefix}:__deleted__:${key}`,
      embed: false,
    });
  }

  close(): void {
    this.client.close();
  }
}

export { Synapse };
