export type SearchMode = "Lex" | "Vec" | "Hybrid";
export interface PutRequest {
    title?: string;
    uri?: string;
    text: string;
    meta?: unknown;
    embed?: boolean;
}
export interface Hit {
    id: number;
    uri: string | null;
    title: string | null;
    text: string;
    score: number;
}
export interface Stats {
    docs: number;
    vecs: number;
}
export declare class Synapse {
    private sockPath;
    private sock;
    private buf;
    private queue;
    private connecting;
    constructor(sockPath?: string);
    private connect;
    private onData;
    private onClose;
    private call;
    ping(): Promise<string>;
    put(req: PutRequest): Promise<number>;
    putBatch(reqs: PutRequest[]): Promise<number[]>;
    search(q: string, opts?: {
        mode?: SearchMode;
        limit?: number;
        embedQuery?: boolean;
    }): Promise<Hit[]>;
    stats(): Promise<Stats>;
    snap(out: string, level?: number): Promise<string>;
    close(): void;
}
