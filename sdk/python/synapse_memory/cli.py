"""synx — CLI wrapper over socket client."""
import json
import sys
import time

from .client import Client, SynapseError


def main():
    args = sys.argv[1:] or ["ping"]
    op = args[0]
    c = Client()
    try:
        if op == "ping":
            print("Pong" if c.ping() else "down")
        elif op == "stats":
            print(json.dumps(c.stats()))
        elif op in ("search", "hybrid", "find", "vec"):
            mode = {"search": "hybrid", "hybrid": "hybrid",
                    "find": "lex", "vec": "vec"}[op]
            hits = c.search(args[1], mode=mode,
                            limit=int(args[2]) if len(args) > 2 else 10)
            for h in hits:
                title = (h.get("title") or h.get("uri") or f"id:{h.get('id')}")[:80]
                score = h.get("score", 0.0)
                snippet = (h.get("text") or "")[:140].replace("\n", " ")
                print(f"{score:.3f}\t{title}\t{snippet}")
        elif op == "put":
            text = sys.stdin.read() if len(args) == 1 else args[1]
            title = None
            for i, a in enumerate(args[1:], 1):
                if a == "--title" and i + 1 < len(args):
                    title = args[i + 1]
            print(c.put(text, title=title))
        elif op == "put-batch":
            items = [json.loads(l) for l in sys.stdin if l.strip()]
            t = time.perf_counter()
            ids = c.put_batch(items)
            print(f"indexed {len(ids)} in {time.perf_counter() - t:.2f}s",
                  file=sys.stderr)
            print(json.dumps(ids))
        elif op == "agent-observe":
            if len(args) < 2:
                raise SynapseError("usage: synx agent-observe AGENT [text]")
            agent = c.agent(args[1])
            text = sys.stdin.read() if len(args) == 2 else args[2]
            print(agent.observe(text))
        elif op == "agent-search":
            if len(args) < 3:
                raise SynapseError("usage: synx agent-search AGENT QUERY [N]")
            agent = c.agent(args[1])
            hits = agent.search_index(args[2], limit=int(args[3]) if len(args) > 3 else 8)
            print(json.dumps(hits, ensure_ascii=False))
        elif op == "agent-get":
            if len(args) < 3:
                raise SynapseError("usage: synx agent-get AGENT ID [ID...]")
            agent = c.agent(args[1])
            docs = agent.get_observations(args[2:])
            print(json.dumps(docs, ensure_ascii=False))
        elif op == "agent-context":
            if len(args) < 3:
                raise SynapseError("usage: synx agent-context AGENT QUERY [TOKENS]")
            agent = c.agent(args[1])
            pack = agent.context_pack(
                args[2],
                token_budget=int(args[3]) if len(args) > 3 else 800,
            )
            print(pack["context"])
        elif op == "agent-feedback":
            if len(args) < 5:
                raise SynapseError("usage: synx agent-feedback AGENT QUERY OUTCOME ID [ID...]")
            agent = c.agent(args[1])
            print(agent.feedback(args[2], args[4:], outcome=args[3]))
        elif op == "bench":
            c.ping()
            t = time.perf_counter()
            for _ in range(100):
                c.ping()
            print(f"ping  100x {(time.perf_counter()-t)*10:.2f}ms/call")
            t = time.perf_counter()
            for _ in range(20):
                c.search("test", limit=5)
            print(f"search 20x {(time.perf_counter()-t)*50:.2f}ms/call")
        else:
            print("usage: synx {ping|stats|search Q [N]|hybrid Q [N]|find Q|vec Q|"
                  "put [text]|put-batch|agent-observe|agent-search|agent-get|"
                  "agent-context|agent-feedback|bench}", file=sys.stderr)
            sys.exit(1)
    except SynapseError as e:
        print(f"synapse error: {e}", file=sys.stderr)
        sys.exit(2)


if __name__ == "__main__":
    main()
