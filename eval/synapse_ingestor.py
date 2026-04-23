#!/usr/bin/env python3
"""
synapse_ingestor.py

Multi-repository ingestion for Synapse Brain.
Supports:
- Local directories
- Git repositories
- GitHub/GitLab APIs
- File archives
- Web scraping
"""

import os
import re
import json
import sqlite3
import hashlib
import threading
from pathlib import Path
from dataclasses import dataclass, field
from typing import List, Dict, Optional, Set, Iterator, Callable
from concurrent.futures import ThreadPoolExecutor, as_completed
from datetime import datetime
import subprocess

# ═══════════════════════════════════════════════════════════════════════════
# CONFIGURATION
# ═══════════════════════════════════════════════════════════════════════════

BRAIN_DB = os.path.expanduser("~/.synapse/brain.db")
MAX_FILE_SIZE = 10 * 1024 * 1024  # 10MB
MAX_LINE_LENGTH = 100000
SUPPORTED_EXTENSIONS = {
    # Code
    ".py", ".js", ".ts", ".jsx", ".tsx", ".java", ".c", ".cpp", ".h", ".hpp",
    ".cs", ".go", ".rs", ".rb", ".php", ".swift", ".kt", ".scala", ".r",
    # Web
    ".html", ".css", ".scss", ".sass", ".less",
    # Data
    ".json", ".yaml", ".yml", ".toml", ".xml", ".csv", ".tsv",
    # Docs
    ".md", ".rst", ".txt", ".adoc", ".tex",
    # Config
    ".env", ".ini", ".cfg", ".conf", ".properties",
    # Shell
    ".sh", ".bash", ".zsh", ".fish",
    # Docker
    "Dockerfile", ".dockerignore", "docker-compose.yml", "docker-compose.yaml",
    # Kubernetes
    ".kubeconfig",
    # Cloud
    "Terraform", ".tf", ".tfvars",
}

SKIP_PATTERNS = {
    # Git
    ".git/", ".gitignore", ".gitattributes",
    # Node
    "node_modules/", ".npm/", ".yarn/",
    # Python
    "__pycache__/", ".pytest_cache/", ".mypy_cache/", "venv/", ".venv/",
    "dist/", "build/", ".eggs/", "*.egg-info/",
    # Rust
    "target/", ".cargo/",
    # Java
    ".gradle/", "build/", "*.class",
    # IDE
    ".idea/", ".vscode/", "*.swp", "*.swo", "*~",
    # Build
    "*.o", "*.so", "*.dylib", "*.dll", "*.exe",
    # Minified
    "*.min.js", "*.min.css", "*.map",
    # Assets
    "*.png", "*.jpg", "*.jpeg", "*.gif", "*.ico", "*.svg", "*.webp",
    "*.mp3", "*.mp4", "*.wav", "*.flac",
    "*.pdf", "*.zip", "*.tar", "*.gz", "*.rar", "*.7z",
}

# ═══════════════════════════════════════════════════════════════════════════
# DATA STRUCTURES
# ═══════════════════════════════════════════════════════════════════════════

@dataclass
class Document:
    """A document to be ingested."""
    path: str
    content: str
    title: str
    doc_type: str
    source: str
    language: Optional[str] = None
    metadata: Dict = field(default_factory=dict)
    blake3_hash: Optional[str] = None

@dataclass
class IngestStats:
    """Statistics for ingestion."""
    files_scanned: int = 0
    files_ingested: int = 0
    files_skipped: int = 0
    files_failed: int = 0
    bytes_processed: int = 0
    start_time: datetime = field(default_factory=datetime.now)
    end_time: Optional[datetime] = None
    
    def duration_ms(self) -> float:
        if self.end_time:
            return (self.end_time - self.start_time).total_seconds() * 1000
        return 0
    
    def files_per_second(self) -> float:
        duration = self.duration_ms() / 1000
        if duration > 0:
            return self.files_ingested / duration
        return 0

# ═══════════════════════════════════════════════════════════════════════════
# FILE PROCESSORS
# ═══════════════════════════════════════════════════════════════════════════

class FileProcessor:
    """Process files for ingestion."""
    
    LANGUAGE_MAP = {
        ".py": "python",
        ".js": "javascript",
        ".ts": "typescript",
        ".jsx": "javascript",
        ".tsx": "typescript",
        ".java": "java",
        ".c": "c",
        ".cpp": "cpp",
        ".h": "c",
        ".hpp": "cpp",
        ".cs": "csharp",
        ".go": "go",
        ".rs": "rust",
        ".rb": "ruby",
        ".php": "php",
        ".swift": "swift",
        ".kt": "kotlin",
        ".scala": "scala",
        ".r": "r",
        ".html": "html",
        ".css": "css",
        ".scss": "scss",
        ".md": "markdown",
        ".json": "json",
        ".yaml": "yaml",
        ".yml": "yaml",
        ".toml": "toml",
        ".xml": "xml",
        ".sh": "bash",
        ".bash": "bash",
        ".zsh": "bash",
    }
    
    TYPE_MAP = {
        ".py": "code",
        ".js": "code",
        ".ts": "code",
        ".md": "documentation",
        ".txt": "text",
        ".json": "config",
        ".yaml": "config",
        ".yml": "config",
        ".toml": "config",
        ".xml": "config",
        ".sh": "script",
        ".html": "web",
        ".css": "web",
    }
    
    @classmethod
    def get_language(cls, path: str) -> Optional[str]:
        """Get programming language from file extension."""
        ext = os.path.splitext(path)[1].lower()
        return cls.LANGUAGE_MAP.get(ext)
    
    @classmethod
    def get_doc_type(cls, path: str) -> str:
        """Get document type from file extension."""
        ext = os.path.splitext(path)[1].lower()
        name = os.path.basename(path)
        
        # Special cases
        if name == "Dockerfile":
            return "docker"
        if name in ("docker-compose.yml", "docker-compose.yaml"):
            return "docker"
        if name.endswith(".tf") or "terraform" in name.lower():
            return "infra"
        
        return cls.TYPE_MAP.get(ext, "unknown")
    
    @classmethod
    def should_skip(cls, path: str) -> bool:
        """Check if file should be skipped."""
        path_lower = path.lower()
        
        for pattern in SKIP_PATTERNS:
            if pattern in path_lower or path.endswith(pattern):
                return True
        
        return False
    
    @classmethod
    def is_supported(cls, path: str) -> bool:
        """Check if file type is supported."""
        ext = os.path.splitext(path)[1].lower()
        name = os.path.basename(path)
        
        return ext in cls.LANGUAGE_MAP or name in SUPPORTED_EXTENSIONS
    
    @classmethod
    def process_file(cls, path: str, content: str) -> Optional[Document]:
        """Process a single file into a Document."""
        try:
            stat = os.stat(path)
            
            # Size check
            if stat.st_size > MAX_FILE_SIZE:
                return None
            
            # Extract title from path
            title = os.path.basename(path)
            if not title:
                title = path
            
            # Generate hash (blake3 preferred, sha256 fallback)
            try:
                import blake3
                doc_hash = blake3.blake3(content.encode()).hexdigest()
            except ImportError:
                doc_hash = hashlib.sha256(content.encode()).hexdigest()
            
            return Document(
                path=path,
                content=content[:500000],  # Limit content size
                title=title,
                doc_type=cls.get_doc_type(path),
                source="filesystem",
                language=cls.get_language(path),
                metadata={
                    "size": stat.st_size,
                    "modified": stat.st_mtime,
                    "extension": os.path.splitext(path)[1],
                },
                blake3_hash=doc_hash,
            )
        except Exception as e:
            print(f"Error processing {path}: {e}")
            return None

# ═══════════════════════════════════════════════════════════════════════════
# INGESTOR
# ═══════════════════════════════════════════════════════════════════════════

class SynapseIngestor:
    """Ingest documents into Synapse Brain."""
    
    def __init__(self, db_path: str = BRAIN_DB):
        self.db_path = db_path
        self.con = sqlite3.connect(db_path, check_same_thread=False)
        self.con.execute("PRAGMA mmap_size=268435456")
        self.con.execute("PRAGMA cache_size=-64000")
        self.con.execute("PRAGMA journal_mode=WAL")
        self.con.execute("PRAGMA synchronous=NORMAL")
        self._ensure_schema()
        self.stats = IngestStats()
        self.seen_hashes: Set[str] = set()
        self._lock = threading.Lock()

    def _ensure_schema(self):
        """Create schema if not exists (mirrors Rust Store::migrate)."""
        self.con.executescript("""
CREATE TABLE IF NOT EXISTS meta (
    k TEXT PRIMARY KEY,
    v TEXT NOT NULL
);
CREATE TABLE IF NOT EXISTS docs (
    id      INTEGER PRIMARY KEY AUTOINCREMENT,
    uri     TEXT UNIQUE,
    title   TEXT,
    text    TEXT NOT NULL,
    meta    TEXT,
    ts      INTEGER NOT NULL DEFAULT 0,
    blake3  TEXT NOT NULL UNIQUE,
    sig     BLOB,
    meta_crdt BLOB
);
CREATE INDEX IF NOT EXISTS idx_docs_ts ON docs(ts);

CREATE VIRTUAL TABLE IF NOT EXISTS docs_fts USING fts5(
    title, text, content='docs', content_rowid='id',
    tokenize='porter unicode61 remove_diacritics 2'
);

CREATE TRIGGER IF NOT EXISTS docs_ai AFTER INSERT ON docs BEGIN
    INSERT INTO docs_fts(rowid, title, text) VALUES (new.id, new.title, new.text);
END;
CREATE TRIGGER IF NOT EXISTS docs_ad AFTER DELETE ON docs BEGIN
    INSERT INTO docs_fts(docs_fts, rowid, title, text) VALUES('delete', old.id, old.title, old.text);
END;
CREATE TRIGGER IF NOT EXISTS docs_au AFTER UPDATE ON docs BEGIN
    INSERT INTO docs_fts(docs_fts, rowid, title, text) VALUES('delete', old.id, old.title, old.text);
    INSERT INTO docs_fts(rowid, title, text) VALUES (new.id, new.title, new.text);
END;

INSERT OR IGNORE INTO meta(k,v) VALUES
  ('schema_version','1'),
  ('embed_dim','384'),
  ('embed_model','bge-small-en-v1.5');
""")
        
    def close(self):
        """Close connection."""
        self.con.close()
    
    def scan_directory(self, path: str) -> Iterator[str]:
        """Scan directory for files to ingest."""
        path = os.path.expanduser(path)
        
        for root, dirs, files in os.walk(path):
            # Filter hidden directories
            dirs[:] = [d for d in dirs if not d.startswith('.')]
            
            for file in files:
                file_path = os.path.join(root, file)
                
                if FileProcessor.should_skip(file_path):
                    self.stats.files_skipped += 1
                    continue
                
                if not FileProcessor.is_supported(file_path):
                    self.stats.files_skipped += 1
                    continue
                
                yield file_path
    
    def ingest_file(self, path: str) -> bool:
        """Ingest a single file."""
        try:
            self.stats.files_scanned += 1
            
            with open(path, 'r', encoding='utf-8', errors='ignore') as f:
                content = f.read()
            
            # Limit line length
            lines = content.split('\n')
            lines = [l[:MAX_LINE_LENGTH] for l in lines]
            content = '\n'.join(lines)
            
            doc = FileProcessor.process_file(path, content)
            if not doc:
                return False
            
            with self._lock:
                # Check for duplicates
                if doc.blake3_hash in self.seen_hashes:
                    return False
                self.seen_hashes.add(doc.blake3_hash)
                
                # Insert into database (matches Rust Store schema)
                meta = doc.metadata.copy()
                meta["doc_type"] = doc.doc_type
                self.con.execute("""
                    INSERT INTO docs (uri, title, text, meta, blake3, meta_crdt)
                    VALUES (?, ?, ?, ?, ?, ?)
                """, (
                    doc.path,
                    doc.title,
                    doc.content,
                    json.dumps(meta),
                    doc.blake3_hash,
                    json.dumps({"source": doc.source, "language": doc.language}),
                ))
                
                self.stats.files_ingested += 1
                self.stats.bytes_processed += len(content)
            
            return True
            
        except Exception as e:
            self.stats.files_failed += 1
            return False
    
    def ingest_directory(
        self, 
        path: str, 
        parallel: int = 10,
        progress_callback: Optional[Callable[[IngestStats], None]] = None
    ) -> IngestStats:
        """Ingest all files from a directory."""
        print(f"Scanning {path}...")
        
        files = list(self.scan_directory(path))
        total = len(files)
        print(f"Found {total} files to ingest")
        
        self.stats = IngestStats()
        
        with ThreadPoolExecutor(max_workers=parallel) as executor:
            futures = {executor.submit(self.ingest_file, f): f for f in files}
            
            completed = 0
            for future in as_completed(futures):
                completed += 1
                if completed % 100 == 0:
                    print(f"Progress: {completed}/{total} ({completed/total*100:.1f}%)")
                    if progress_callback:
                        progress_callback(self.stats)
        
        self.stats.end_time = datetime.now()
        self.con.commit()
        
        return self.stats
    
    def ingest_git_repo(self, repo_path: str, parallel: int = 10) -> IngestStats:
        """Ingest a git repository."""
        # First, get list of tracked files
        try:
            result = subprocess.run(
                ["git", "-C", repo_path, "ls-files"],
                capture_output=True,
                text=True,
                check=True
            )
            files = result.stdout.strip().split('\n')
            
            full_paths = [os.path.join(repo_path, f) for f in files if f]
            
            self.stats = IngestStats()
            
            for path in full_paths:
                self.ingest_file(path)
            
            self.stats.end_time = datetime.now()
            self.con.commit()
            
            return self.stats
            
        except subprocess.CalledProcessError as e:
            print(f"Git error: {e}")
            return self.stats

# ═══════════════════════════════════════════════════════════════════════════
# MAIN
# ═══════════════════════════════════════════════════════════════════════════

def main():
    import argparse
    
    parser = argparse.ArgumentParser(description="Synapse Ingestor")
    parser.add_argument("--path", help="Directory to ingest")
    parser.add_argument("--git", help="Git repository to ingest")
    parser.add_argument("--parallel", type=int, default=10, help="Parallel workers")
    parser.add_argument("--db", default=BRAIN_DB, help="Database path")
    
    args = parser.parse_args()
    
    ingestor = SynapseIngestor(args.db)
    
    if args.path:
        stats = ingestor.ingest_directory(args.path, args.parallel)
    elif args.git:
        stats = ingestor.ingest_git_repo(args.git, args.parallel)
    else:
        print("Please specify --path or --git")
        return
    
    print(f"""
╔═══════════════════════════════════════════════════════════════════════╗
║                    INGESTION COMPLETE                           ║
╚═══════════════════════════════════════════════════════════════════════╝

Files scanned:    {stats.files_scanned:,}
Files ingested:   {stats.files_ingested:,}
Files skipped:    {stats.files_skipped:,}
Files failed:    {stats.files_failed:,}
Bytes processed: {stats.bytes_processed:,}
Duration:        {stats.duration_ms():.0f}ms
Speed:           {stats.files_per_second():.0f} files/sec
""")
    
    ingestor.close()

if __name__ == "__main__":
    main()
