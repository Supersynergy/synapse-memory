"""
04-mcp-server/validate.py
Validates synapse-mcp is reachable and tools work end-to-end.

Usage:
  # Start server first:
  synapse-mcp --sock /tmp/synapse.sock --db /tmp/mcp-test.db

  # Then run:
  python validate.py
"""

import json
import os
import socket
import sys
import time

SOCK = os.environ.get("SYNAPSE_SOCK", "/tmp/synapse.sock")


def jsonrpc(sock_path: str, method: str, params: dict) -> dict:
    payload = json.dumps({"jsonrpc": "2.0", "id": 1, "method": method, "params": params}) + "\n"
    with socket.socket(socket.AF_UNIX, socket.SOCK_STREAM) as s:
        s.connect(sock_path)
        s.sendall(payload.encode())
        buf = b""
        while True:
            chunk = s.recv(4096)
            if not chunk:
                break
            buf += chunk
            try:
                return json.loads(buf.decode())
            except json.JSONDecodeError:
                continue
    return {}


def check(label: str, result: dict) -> bool:
    ok = "error" not in result
    status = "PASS" if ok else "FAIL"
    print(f"  [{status}] {label}")
    if not ok:
        print(f"         {result.get('error')}")
    return ok


def main():
    if not os.path.exists(SOCK):
        print(f"[error] Socket not found: {SOCK}")
        print("        Start server first: synapse-mcp --sock /tmp/synapse.sock --db /tmp/mcp-test.db")
        sys.exit(1)

    print(f"Validating synapse-mcp at {SOCK}\n")
    passed = 0
    total = 0

    # 1. list tools
    total += 1
    r = jsonrpc(SOCK, "tools/list", {})
    if check("tools/list returns tool names", r):
        names = [t["name"] for t in r.get("result", {}).get("tools", [])]
        print(f"         tools: {names}")
        passed += 1

    # 2. memory_save
    total += 1
    r = jsonrpc(SOCK, "tools/call", {
        "name": "memory_save",
        "arguments": {"text": "user prefers dark mode", "tags": ["preference", "ui"]}
    })
    if check("memory_save stores a memory", r):
        passed += 1

    # 3. memory_search
    time.sleep(0.1)
    total += 1
    r = jsonrpc(SOCK, "tools/call", {
        "name": "memory_search",
        "arguments": {"query": "dark mode", "k": 3}
    })
    if check("memory_search retrieves stored memory", r):
        passed += 1

    # 4. memory_recent
    total += 1
    r = jsonrpc(SOCK, "tools/call", {
        "name": "memory_recent",
        "arguments": {"n": 5}
    })
    if check("memory_recent returns recent memories", r):
        passed += 1

    # 5. put + search
    total += 1
    r = jsonrpc(SOCK, "tools/call", {
        "name": "put",
        "arguments": {"text": "Synapse is faster than Pinecone", "title": "bench"}
    })
    if check("put appends a doc", r):
        passed += 1

    total += 1
    r = jsonrpc(SOCK, "tools/call", {
        "name": "search",
        "arguments": {"query": "Pinecone", "mode": "lex", "limit": 5}
    })
    if check("search finds the doc", r):
        passed += 1

    print(f"\nResult: {passed}/{total} passed")
    sys.exit(0 if passed == total else 1)


if __name__ == "__main__":
    main()
