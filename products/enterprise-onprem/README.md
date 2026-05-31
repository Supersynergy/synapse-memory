# Synapse Enterprise On-Prem

Private local/on-prem agent memory and freshness layer.

## Product Promise

Give enterprise agent teams persistent memory without sending source code,
prompts, or retrieved context to a hosted memory SaaS.

## Included

- Docker runtime files.
- Enterprise security PDF and source markdown.
- Homebrew, K8s, npm wrapper, and release docs.
- Deployment-oriented manifest.

## Current Product Shape

This is a sales/deployment package. Before aggressive enterprise sales, add a
formal threat model, signed release artifacts, SBOM, and reproducible builds.

## Verify

```bash
products/enterprise-onprem/scripts/verify.sh
```

Set `RUN_DOCKER=1` to also build the local Docker image.
