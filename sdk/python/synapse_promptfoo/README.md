# synapse-promptfoo

promptfoo eval provider for [synapse](https://github.com/Supersynergy/synapse) RAG benchmarking.

Interface spec: https://www.promptfoo.dev/docs/providers/custom-api

## Install

```bash
pip install synapse-promptfoo
```

## Usage

In your `promptfoo.yaml`:

```yaml
providers:
  - id: "python:synapse_promptfoo.provider:call"
    config:
      sock_path: "/tmp/synapse.sock"
      mode: "Hybrid"
      limit: 5
```

Run eval:

```bash
promptfoo eval -c examples/promptfoo.yaml
```

## bench/promptfoo.yaml

A ready-made RAG quality benchmark is in `examples/promptfoo.yaml`. Tests Paris/France recall, Python topic recall, and general retrieval coverage.
