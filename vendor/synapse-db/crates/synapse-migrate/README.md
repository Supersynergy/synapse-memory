# synapse-migrate

Import vectors from external stores into a Synapse `.synx` brain file.

## Supported Sources

| Source    | URI scheme                                                  | Auth env          | Notes                              |
|-----------|-------------------------------------------------------------|-------------------|------------------------------------|
| Qdrant    | `qdrant://host:6333/collection`                             | —                 | REST scroll API                    |
| Chroma    | `chroma:///path/to/chroma_dir/collection_name`             | —                 | SQLite direct read                 |
| LanceDB   | `lancedb:///path/to/db/table_name`                          | —                 | Skeleton (fragment placeholders)   |
| Pinecone  | `pinecone://<host>/[namespace]`                             | `PINECONE_API_KEY` | list → fetch; pagination_token    |
| Weaviate  | `weaviate://host:8080/ClassName`                            | `WEAVIATE_API_KEY` (optional) | GraphQL Get                |

## Usage

```bash
# Qdrant
synapse-migrate --from qdrant://localhost:6333/my_collection --to brain.db

# Chroma
synapse-migrate --from chroma:///data/chroma/my_collection --to brain.db

# Pinecone (set PINECONE_API_KEY first)
export PINECONE_API_KEY=pc-xxx
synapse-migrate --from "pinecone://my-index-abc.svc.us-east1-gcp.pinecone.io/default" --to brain.db

# Weaviate (unauthenticated local)
synapse-migrate --from weaviate://localhost:8080/Article --to brain.db

# Weaviate (authenticated)
export WEAVIATE_API_KEY=my-key
synapse-migrate --from weaviate://my-cluster.weaviate.network/Article --to brain.db

# Resume from offset
synapse-migrate --from qdrant://localhost:6333/my_collection --offset 5000 --to brain.db
```

## Feature Gates

Features are compile-time hints only (no hard deps change). All sources compile by default.

- `migrate-pinecone` — tag for Pinecone source
- `migrate-weaviate` — tag for Weaviate source
- `migrate-qdrant` — tag for Qdrant source
- `migrate-lancedb` — tag for LanceDB source
- `migrate-chroma` — tag for Chroma source

## Pinecone Notes

- Uses `/vectors/list` (pagination via `pagination_token`) then `/vectors/fetch` per batch
- Reads `metadata.text` or `metadata.content` as document body
- Reads `metadata.uri`/`metadata.url` and `metadata.title` when present
- Original vector values are preserved

## Weaviate Notes

- Uses GraphQL `Get { ClassName(limit, offset) { _additional { id vector } text content body title uri url } }`
- Fetches `_additional.vector` for vector preservation
- Unauthenticated by default; set `WEAVIATE_API_KEY` for cloud instances

## Next Sources

redis-search · milvus
