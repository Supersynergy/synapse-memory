export interface SearchHit {
  id: number
  uri: string | null
  title: string | null
  text: string
  score: number
}

export declare class Synapse {
  constructor(path: string)
  /** Insert a document. Returns the assigned doc id. */
  put(id: string, text: string, metaJson?: string | null): Promise<number>
  /** Full-text (lexical) search. Returns top-k hits. */
  search(query: string, limit: number): Promise<SearchHit[]>
  /** Hybrid search (lexical + vector). Requires a pre-computed query embedding. */
  searchHybrid(query: string, embedding: number[], limit: number): Promise<SearchHit[]>
  /** Flush WAL and close. */
  close(): Promise<void>
}
