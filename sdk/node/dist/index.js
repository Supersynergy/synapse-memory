import { createConnection } from "node:net";
import { encode, decode } from "@msgpack/msgpack";
import { Buffer } from "node:buffer";
export class Synapse {
    sockPath;
    sock = null;
    buf = Buffer.alloc(0);
    queue = [];
    connecting = null;
    constructor(sockPath = "/tmp/synapse.sock") {
        this.sockPath = sockPath;
    }
    async connect() {
        if (this.sock)
            return;
        if (this.connecting)
            return this.connecting;
        this.connecting = new Promise((resolve, reject) => {
            const s = createConnection(this.sockPath);
            s.once("connect", () => {
                this.sock = s;
                s.on("data", (d) => this.onData(d));
                s.on("error", (e) => this.onClose(e));
                s.on("close", () => this.onClose());
                resolve();
            });
            s.once("error", reject);
        });
        return this.connecting;
    }
    onData(d) {
        this.buf = Buffer.concat([this.buf, d]);
        while (this.buf.length >= 4) {
            const n = this.buf.readUInt32LE(0);
            if (this.buf.length < 4 + n)
                break;
            const body = this.buf.subarray(4, 4 + n);
            this.buf = this.buf.subarray(4 + n);
            const resolver = this.queue.shift();
            if (resolver)
                resolver(Buffer.from(body));
        }
    }
    onClose(err) {
        this.sock = null;
        this.connecting = null;
        const q = this.queue;
        this.queue = [];
        for (const r of q)
            r(Buffer.from(encode({ Err: err?.message ?? "closed" })));
    }
    async call(req) {
        await this.connect();
        return new Promise((resolve, reject) => {
            const body = encode(req);
            const hdr = Buffer.alloc(4);
            hdr.writeUInt32LE(body.byteLength, 0);
            this.queue.push((buf) => {
                try {
                    const resp = decode(buf);
                    if (typeof resp === "string")
                        return resolve(resp);
                    if ("Err" in resp)
                        return reject(new Error(String(resp.Err)));
                    if ("Id" in resp)
                        return resolve(resp.Id);
                    if ("Ids" in resp)
                        return resolve(resp.Ids);
                    if ("Hits" in resp)
                        return resolve(resp.Hits);
                    if ("Stats" in resp)
                        return resolve(resp.Stats);
                    if ("Pong" in resp)
                        return resolve("pong");
                    if ("Ok" in resp)
                        return resolve("ok");
                    resolve(resp);
                }
                catch (e) {
                    reject(e);
                }
            });
            this.sock.write(hdr);
            this.sock.write(Buffer.from(body));
        });
    }
    async ping() {
        return this.call({ op: "Ping" });
    }
    async put(req) {
        return this.call({
            op: "Put",
            args: { title: req.title ?? null, uri: req.uri ?? null, text: req.text, meta: req.meta ?? null, embed: !!req.embed },
        });
    }
    async putBatch(reqs) {
        return this.call({
            op: "PutBatch",
            args: reqs.map((r) => ({ title: r.title ?? null, uri: r.uri ?? null, text: r.text, meta: r.meta ?? null, embed: !!r.embed })),
        });
    }
    async search(q, opts = {}) {
        return this.call({
            op: "Search",
            args: { mode: opts.mode ?? "Lex", q, limit: opts.limit ?? 10, embed_query: opts.embedQuery ?? false },
        });
    }
    async stats() {
        return this.call({ op: "Stats" });
    }
    async snap(out, level = 3) {
        return this.call({ op: "Snap", args: { out, level } });
    }
    close() {
        this.sock?.end();
        this.sock = null;
    }
}
