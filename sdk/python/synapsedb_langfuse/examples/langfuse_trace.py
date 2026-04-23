"""
Langfuse tracing example with SynapseRetriever.
Run: python examples/langfuse_trace.py
Requires: synapsed at /tmp/synapsedb.sock, LANGFUSE_SECRET_KEY + LANGFUSE_PUBLIC_KEY in env
"""
import os
from synapsedb_langfuse import SynapseRetriever

# Optional: real Langfuse client. Falls back to no-op if not configured.
try:
    from langfuse import Langfuse
    lf = Langfuse(
        secret_key=os.environ.get("LANGFUSE_SECRET_KEY", ""),
        public_key=os.environ.get("LANGFUSE_PUBLIC_KEY", ""),
        host=os.environ.get("LANGFUSE_HOST", "https://cloud.langfuse.com"),
    )
except ImportError:
    lf = None
    print("langfuse not installed — tracing disabled, search still works")

retriever = SynapseRetriever(langfuse=lf, mode="Hybrid")

# Put some test data first (requires synapsed running)
import socket, struct, msgpack

def put_doc(text: str, uri: str) -> None:
    s = socket.socket(socket.AF_UNIX, socket.SOCK_STREAM)
    try:
        s.connect("/tmp/synapsedb.sock")
        body = msgpack.packb({"op": "Put", "args": {"text": text, "uri": uri, "title": None, "meta": None, "embed": True}})
        s.sendall(struct.pack("<I", len(body)) + body)
        head = b""
        while len(head) < 4:
            head += s.recv(4 - len(head))
        n = struct.unpack("<I", head)[0]
        resp = b""
        while len(resp) < n:
            resp += s.recv(n - len(resp))
    finally:
        s.close()

put_doc("Paris is the capital of France.", "langfuse-demo:1")
put_doc("Berlin is the capital of Germany.", "langfuse-demo:2")

hits = retriever.search("capital of France", limit=3, trace_id="demo-trace-001")
print(f"Found {len(hits)} hits:")
for h in hits:
    print(f"  [{h.get('score', 0):.3f}] {h.get('text', '')[:60]}")

retriever.close()
