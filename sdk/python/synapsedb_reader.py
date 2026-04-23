"""
Synapse `.synx` conformance reader — Phase 3 Track (d).

Minimal, zero-dep (stdlib + zstandard) Python reader. Round-trip compatible
with the Rust v0.2 writer: verifies magic bytes, parses header+footer, walks
every chunk, decodes zstd payloads, and checks BLAKE3 content hashes.

Usage:
    from synapse_reader import SynxReader
    r = SynxReader("brain.synx")
    print(r.manifest["stats"], len(r.chunks))
    for i, c in enumerate(r.chunks):
        body = r.decode(i)
        ...

Dependencies:
    pip install zstandard blake3
"""
from __future__ import annotations

import json
import struct
from dataclasses import dataclass
from pathlib import Path
from typing import List, Optional

try:
    import blake3 as _blake3  # type: ignore
except ImportError as e:
    raise SystemExit("pip install blake3  (required for hash verification)") from e
try:
    import zstandard as _zstd  # type: ignore
except ImportError as e:
    raise SystemExit("pip install zstandard  (required for chunk codec)") from e


MAGIC = b"SYNX"
FOOTER_MAGIC = b"XNYS"
HEADER_SIZE = 64
FOOTER_SIZE = 256
CHUNK_FRAME_HEADER = 16   # total_len:u32 | kind:u16 | codec:u8 | flags:u8 | reserved:[u8;8]
CHUNK_HASH_LEN = 32
CHUNK_ULEN = 8

CHUNK_KIND_NAMES = {
    0x01: "RowBatch",
    0x02: "TextBlob",
    0x03: "FtsSegment",
    0x04: "VecIndex",
    0x05: "VecPayload",
    0x06: "CRDTOpsLog",
    0x07: "SchemaDef",
    0x08: "MerkleNode",
    0xFF: "Tombstone",
}

CODEC_NAMES = {0: "Raw", 1: "Zstd"}


@dataclass
class SynxHeader:
    version: int
    flags: int
    manifest_offset: int
    footer_offset: int
    created_unix: int
    creator_uuid: bytes


@dataclass
class SynxFooter:
    manifest_hash: bytes
    signature: Optional[bytes]
    pubkey: Optional[bytes]


@dataclass
class Chunk:
    kind: int
    codec: int
    flags: int
    hash: bytes
    uncompressed_len: int
    payload: bytes

    @property
    def kind_name(self) -> str:
        return CHUNK_KIND_NAMES.get(self.kind, f"Unknown({self.kind:#04x})")

    @property
    def codec_name(self) -> str:
        return CODEC_NAMES.get(self.codec, f"Unknown({self.codec})")


class SynxReader:
    def __init__(self, path: str | Path) -> None:
        self.path = Path(path)
        self.bytes = self.path.read_bytes()
        if self.bytes[:4] != MAGIC:
            raise ValueError(f"bad magic — not a .synx file: {self.path}")
        self.header = self._parse_header()
        self.footer = self._parse_footer()
        self.manifest = self._parse_manifest()
        self.chunks = self._index_chunks()

    # --- parsing -----------------------------------------------------------

    def _parse_header(self) -> SynxHeader:
        buf = self.bytes[:HEADER_SIZE]
        version = struct.unpack_from("<H", buf, 4)[0]
        if version != 2:
            raise ValueError(f"unsupported .synx version: {version}")
        flags = struct.unpack_from("<H", buf, 6)[0]
        manifest_offset = struct.unpack_from("<Q", buf, 16)[0]
        footer_offset = struct.unpack_from("<Q", buf, 24)[0]
        created_unix = struct.unpack_from("<Q", buf, 32)[0]
        creator_uuid = bytes(buf[40:56])
        return SynxHeader(
            version=version,
            flags=flags,
            manifest_offset=manifest_offset,
            footer_offset=footer_offset,
            created_unix=created_unix,
            creator_uuid=creator_uuid,
        )

    def _parse_footer(self) -> SynxFooter:
        buf = self.bytes[-FOOTER_SIZE:]
        if buf[:4] != FOOTER_MAGIC:
            raise ValueError("bad footer magic")
        manifest_hash = bytes(buf[4:36])
        signed = any(buf[36:100])
        signature = bytes(buf[36:100]) if signed else None
        pubkey = bytes(buf[100:132]) if signed else None
        return SynxFooter(manifest_hash=manifest_hash, signature=signature, pubkey=pubkey)

    def _read_chunk_at(self, offset: int) -> Chunk:
        total_len, kind, codec, flags = struct.unpack_from("<IHBB", self.bytes, offset)
        hash_off = offset + CHUNK_FRAME_HEADER
        ulen_off = hash_off + CHUNK_HASH_LEN
        payload_off = ulen_off + CHUNK_ULEN
        payload_len = total_len - (CHUNK_FRAME_HEADER + CHUNK_HASH_LEN + CHUNK_ULEN)
        chunk_hash = bytes(self.bytes[hash_off:hash_off + CHUNK_HASH_LEN])
        uncompressed_len = struct.unpack_from("<Q", self.bytes, ulen_off)[0]
        payload = bytes(self.bytes[payload_off:payload_off + payload_len])
        return Chunk(
            kind=kind,
            codec=codec,
            flags=flags,
            hash=chunk_hash,
            uncompressed_len=uncompressed_len,
            payload=payload,
        )

    def _parse_manifest(self) -> dict:
        chunk = self._read_chunk_at(self.header.manifest_offset)
        if chunk.hash != self.footer.manifest_hash:
            raise ValueError("manifest hash mismatch")
        body = self._decode_payload(chunk)
        return json.loads(body)

    def _index_chunks(self) -> List[Chunk]:
        out: List[Chunk] = []
        for entry in self.manifest["chunks"]:
            out.append(self._read_chunk_at(int(entry["offset"])))
        return out

    # --- codec + verification ---------------------------------------------

    def _decode_payload(self, c: Chunk) -> bytes:
        if c.codec == 0:
            data = c.payload
        elif c.codec == 1:
            # Rust-side zstd::encode_all does not emit a content-size header,
            # so use stream decompression with the known uncompressed_len hint.
            data = _zstd.ZstdDecompressor().decompress(
                c.payload, max_output_size=c.uncompressed_len
            )
        else:
            raise ValueError(f"unknown codec {c.codec}")
        digest = _blake3.blake3(data).digest()
        if bytes(digest) != c.hash:
            raise ValueError("chunk hash mismatch — corruption")
        if len(data) != c.uncompressed_len:
            raise ValueError("uncompressed length mismatch")
        return data

    def decode(self, idx: int) -> bytes:
        return self._decode_payload(self.chunks[idx])

    # --- conformance checks -----------------------------------------------

    def verify_all(self) -> dict:
        """Run every conformance check a reference reader should. Returns a report."""
        verified = 0
        total_uncompressed = 0
        kinds: dict = {}
        for i, c in enumerate(self.chunks):
            data = self._decode_payload(c)
            verified += 1
            total_uncompressed += len(data)
            kinds[c.kind_name] = kinds.get(c.kind_name, 0) + 1
        return {
            "path": str(self.path),
            "version": self.header.version,
            "flags": self.header.flags,
            "signed": self.footer.signature is not None,
            "chunks_verified": verified,
            "total_uncompressed_bytes": total_uncompressed,
            "kinds": kinds,
            "manifest_hash": self.footer.manifest_hash.hex(),
        }


if __name__ == "__main__":
    import sys

    if len(sys.argv) != 2:
        print("usage: synapse_reader.py <path.synx>", file=sys.stderr)
        sys.exit(2)
    r = SynxReader(sys.argv[1])
    rep = r.verify_all()
    print(json.dumps(rep, indent=2))
