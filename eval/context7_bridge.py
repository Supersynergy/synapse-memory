#!/usr/bin/env python3
"""
context7_bridge.py

Context7-like interface for Synapse Brain.

Provides:
- Multi-file context retrieval
- Cross-reference analysis
- Chunking strategies
- Query expansion
"""

import sqlite3
import json
import os
from typing import List, Dict, Optional, Tuple
from dataclasses import dataclass, field
from pathlib import Path

BRAIN_DB = os.path.expanduser("~/.synapse/brain.db")

# ═══════════════════════════════════════════════════════════════════════════
# CONFIGURATION
# ═══════════════════════════════════════════════════════════════════════════

CHUNK_STRATEGIES = {
    "page": {"tokens": 1024, "overlap": 128},
    "sentence": {"tokens": 512, "overlap": 64},
    "paragraph": {"tokens": 768, "overlap": 96},
    "token": {"tokens": 512, "overlap": 50},
}

DEFAULT_STRATEGY = "page"

# ═══════════════════════════════════════════════════════════════════════════
# DATA STRUCTURES
# ═══════════════════════════════════════════════════════════════════════════

@dataclass
class ContextChunk:
    """A context chunk for retrieval."""
    doc_id: int
    title: str
    content: str
    chunk_index: int
    total_chunks: int
    relevance_score: float = 0.0
    references: List[str] = field(default_factory=list)

@dataclass
class Context7Response:
    """Context7-style response with chunks."""
    query: str
    chunks: List[ContextChunk]
    total_chunks: int
    context_window: int
    citations: List[Dict] = field(default_factory=list)

# ═══════════════════════════════════════════════════════════════════════════
# CHUNKER
# ═══════════════════════════════════════════════════════════════════════════

class TextChunker:
    """Split text into chunks for retrieval."""
    
    def __init__(self, strategy: str = DEFAULT_STRATEGY):
        self.strategy = CHUNK_STRATEGIES.get(strategy, CHUNK_STRATEGIES["page"])
        self.chunk_size = self.strategy["tokens"] * 4  # Approximate chars
        self.overlap = self.strategy["overlap"] * 4
    
    def chunk_text(self, text: str, doc_id: int, title: str) -> List[ContextChunk]:
        """Split text into chunks."""
        chunks = []
        
        if not text:
            return chunks
        
        # Simple character-based chunking
        chars = list(text)
        total_len = len(chars)
        chunk_index = 0
        
        start = 0
        while start < total_len:
            end = min(start + self.chunk_size, total_len)
            
            # Try to break at sentence boundary
            if end < total_len:
                # Look for sentence end
                for i in range(end, max(start, end - 200), -1):
                    if chars[i-1] in '.!?':
                        end = i
                        break
            
            chunk_text = ''.join(chars[start:end])
            
            chunks.append(ContextChunk(
                doc_id=doc_id,
                title=title,
                content=chunk_text.strip(),
                chunk_index=chunk_index,
                total_chunks=-1,  # Will be set later
            ))
            
            chunk_index += 1
            start = end - self.overlap if end < total_len else end
        
        # Update total chunks
        for chunk in chunks:
            chunk.total_chunks = len(chunks)
        
        return chunks
    
    def chunk_by_lines(self, text: str, doc_id: int, title: str, lines_per_chunk: int = 50) -> List[ContextChunk]:
        """Split text by lines."""
        lines = text.split('\n')
        chunks = []
        
        for i in range(0, len(lines), lines_per_chunk):
            chunk_lines = lines[i:i + lines_per_chunk]
            chunks.append(ContextChunk(
                doc_id=doc_id,
                title=title,
                content='\n'.join(chunk_lines),
                chunk_index=i // lines_per_chunk,
                total_chunks=len(lines) // lines_per_chunk + 1,
            ))
        
        return chunks

# ═══════════════════════════════════════════════════════════════════════════
# CONTEXT7 BRIDGE
# ═══════════════════════════════════════════════════════════════════════════

class Context7Bridge:
    """
    Context7-like interface for Synapse Brain.
    
    Provides:
    - Multi-file context retrieval
    - Cross-reference analysis
    - Chunked retrieval
    - Citation tracking
    """
    
    def __init__(self, db_path: str = BRAIN_DB):
        self.db_path = db_path
        self.con = sqlite3.connect(db_path)
        self.con.execute("PRAGMA mmap_size=268435456")
        self.con.execute("PRAGMA cache_size=-64000")
        self.chunker = TextChunker()
    
    def close(self):
        """Close connection."""
        self.con.close()
    
    def retrieve(
        self,
        query: str,
        mode: str = "hybrid",
        limit: int = 10,
        context_window: int = 3,
        include_references: bool = True
    ) -> Context7Response:
        """
        Retrieve Context7-style response.
        
        Args:
            query: Search query
            mode: Search mode (fts5, vector, hybrid)
            limit: Number of documents to retrieve
            context_window: Number of chunks to include per document
            include_references: Include cross-references
        
        Returns:
            Context7Response with chunks and citations
        """
        # Search for documents
        if mode == "fts5":
            docs = self._search_fts5(query, limit)
        elif mode == "vector":
            docs = self._search_vector(query, limit)
        else:  # hybrid
            docs = self._search_hybrid(query, limit)
        
        # Build chunks
        chunks = []
        citations = []
        
        for rank, (doc_id, title, text, score) in enumerate(docs):
            # Get chunks for this document
            doc_chunks = self.chunker.chunk_text(text, doc_id, title)
            
            # Select relevant chunks
            selected = self._select_relevant_chunks(query, doc_chunks, context_window)
            
            for chunk in selected:
                chunk.relevance_score = score
                chunks.append(chunk)
            
            # Build citation
            citations.append({
                "doc_id": doc_id,
                "title": title,
                "rank": rank + 1,
                "score": score,
                "chunks": len(selected),
            })
        
        return Context7Response(
            query=query,
            chunks=chunks[:limit * context_window],
            total_chunks=len(chunks),
            context_window=context_window,
            citations=citations,
        )
    
    def _search_fts5(self, query: str, limit: int) -> List[Tuple]:
        """FTS5 search."""
        try:
            return self.con.execute("""
                SELECT d.id, d.title, d.text, bm25(docs_fts) as score
                FROM docs_fts f 
                JOIN docs d ON d.rowid = f.rowid
                WHERE docs_fts MATCH ?
                ORDER BY score
                LIMIT ?
            """, (query, limit)).fetchall()
        except:
            return []
    
    def _search_vector(self, query: str, limit: int) -> List[Tuple]:
        """Vector search (placeholder)."""
        # TODO: Implement vector search
        return []
    
    def _search_hybrid(self, query: str, limit: int) -> List[Tuple]:
        """Hybrid search combining FTS5 and vector."""
        return self._search_fts5(query, limit)
    
    def _select_relevant_chunks(
        self,
        query: str,
        chunks: List[ContextChunk],
        window: int
    ) -> List[ContextChunk]:
        """Select most relevant chunks."""
        if not chunks:
            return []
        
        query_terms = set(query.lower().split())
        
        scored_chunks = []
        for chunk in chunks:
            content_lower = chunk.content.lower()
            
            # Score by term frequency
            score = sum(1 for term in query_terms if term in content_lower)
            
            # Boost if query terms appear early
            if content_lower[:500]:
                early_terms = set(content_lower[:500].split())
                score += sum(2 for term in query_terms if term in early_terms)
            
            scored_chunks.append((score, chunk))
        
        # Sort by score and take top window
        scored_chunks.sort(key=lambda x: -x[0])
        return [chunk for _, chunk in scored_chunks[:window]]
    
    def retrieve_with_context(
        self,
        query: str,
        file_path: Optional[str] = None,
        language: Optional[str] = None,
        time_range: Optional[Tuple[str, str]] = None,
        limit: int = 20
    ) -> Context7Response:
        """
        Advanced retrieval with filters.
        """
        sql_parts = [
            "SELECT d.id, d.title, d.text, bm25(docs_fts) as score",
            "FROM docs_fts f",
            "JOIN docs d ON d.rowid = f.rowid",
            "WHERE docs_fts MATCH ?"
        ]
        params = [query]
        
        if file_path:
            sql_parts.append("AND d.uri LIKE ?")
            params.append(f"%{file_path}%")
        
        if language:
            sql_parts.append("AND d.doc_type = ?")
            params.append(language)
        
        sql_parts.append("ORDER BY score LIMIT ?")
        params.append(limit)
        
        try:
            docs = self.con.execute(" ".join(sql_parts), params).fetchall()
        except:
            docs = []
        
        chunks = []
        for doc_id, title, text, score in docs:
            doc_chunks = self.chunker.chunk_text(text, doc_id, title)
            for chunk in doc_chunks[:3]:  # Limit chunks per doc
                chunk.relevance_score = score
                chunks.append(chunk)
        
        return Context7Response(
            query=query,
            chunks=chunks,
            total_chunks=len(chunks),
            context_window=3,
            citations=[{"doc_id": d[0], "title": d[1], "score": d[3]} for d in docs],
        )
    
    def get_cross_references(self, doc_id: int) -> List[Dict]:
        """Get cross-references for a document."""
        try:
            doc = self.con.execute(
                "SELECT title, text FROM docs WHERE id = ?", (doc_id,)
            ).fetchone()
            
            if not doc:
                return []
            
            title, text = doc
            refs = []
            
            # Look for file references
            import re
            file_patterns = [
                r'import\s+["\']([^"\']+)["\']',
                r'from\s+["\']([^"\']+)["\']',
                r'require\s*\(["\']([^"\']+)["\']',
                r'include\s+["\']([^"\']+)["\']',
                r'\[([^\]]+\.(py|js|ts|java|cpp|h|c))\]\([^)]+\)',
            ]
            
            for pattern in file_patterns:
                matches = re.findall(pattern, text)
                for match in matches:
                    refs.append({
                        "type": "import",
                        "target": match,
                        "source": title,
                    })
            
            # Look for URLs
            url_pattern = r'https?://[^\s<>"{}|\\^`\[\]]+'
            urls = re.findall(url_pattern, text)
            for url in urls:
                refs.append({
                    "type": "url",
                    "target": url,
                    "source": title,
                })
            
            return refs
            
        except Exception as e:
            return []

# ═══════════════════════════════════════════════════════════════════════════
# MAIN
# ═══════════════════════════════════════════════════════════════════════════

def main():
    import argparse
    
    parser = argparse.ArgumentParser(description="Context7 Bridge")
    parser.add_argument("query", help="Search query")
    parser.add_argument("--mode", choices=["fts5", "vector", "hybrid"], default="hybrid")
    parser.add_argument("--limit", type=int, default=10)
    parser.add_argument("--context-window", type=int, default=3)
    
    args = parser.parse_args()
    
    bridge = Context7Bridge()
    
    response = bridge.retrieve(
        query=args.query,
        mode=args.mode,
        limit=args.limit,
        context_window=args.context_window,
    )
    
    print(f"# Query: {response.query}")
    print(f"# Chunks: {response.total_chunks}")
    print(f"# Citations: {len(response.citations)}")
    print()
    
    for i, chunk in enumerate(response.chunks[:5]):
        print(f"## Chunk {i + 1} ({chunk.relevance_score:.2f})")
        print(f"**{chunk.title}**")
        print(f"```\n{chunk.content[:500]}...\n```")
        print()
    
    for cite in response.citations[:5]:
        print(f"[{cite['rank']}] {cite['title']} (score: {cite['score']:.2f})")
    
    bridge.close()

if __name__ == "__main__":
    main()
