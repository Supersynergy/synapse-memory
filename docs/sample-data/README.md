# Sample Data

Ready-to-ingest datasets for Synapse demos.

| File | Rows | Description |
|------|------|-------------|
| `wiki-100.csv` | 100 | Tech encyclopedia — DB, ML, Rust, TS concepts |
| `code-100.jsonl` | 100 | Code snippets — Rust, Python, TypeScript, SQL, Bash |
| `images-10/` | 10 | Placeholder JPEGs for multimodal demo |

## Ingest wiki-100.csv

```bash
# CLI: one doc per row (uses title+text)
while IFS=, read -r id title text; do
  [[ "$id" == "id" ]] && continue  # skip header
  synapse put -f brain.db --uri "wiki:$id" --title "$title" --text "$text"
done < wiki-100.csv

# Or use put-batch if available:
# synapse put-batch --file wiki-100.csv --format csv -f brain.db
```

## Ingest code-100.jsonl

```bash
while IFS= read -r line; do
  id=$(echo "$line" | jq -r .id)
  title=$(echo "$line" | jq -r .title)
  code=$(echo "$line" | jq -r .code)
  synapse put -f brain.db --uri "code:$id" --title "$title" --text "$code"
done < code-100.jsonl
```

## Python one-liner

```python
import csv, subprocess, json

# wiki
with open("wiki-100.csv") as f:
    for row in csv.DictReader(f):
        subprocess.run(["synapse", "-f", "brain.db", "put",
                       "--uri", f"wiki:{row['id']}",
                       "--title", row["title"],
                       "--text", row["text"]])

# code
with open("code-100.jsonl") as f:
    for line in f:
        d = json.loads(line)
        subprocess.run(["synapse", "-f", "brain.db", "put",
                       "--uri", f"code:{d['id']}",
                       "--title", d["title"],
                       "--text", d["code"]])
```
