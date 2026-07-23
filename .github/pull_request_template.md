<!-- thanks for sending a PR — please keep the diff focused -->

## What this changes

<!-- one or two sentences -->

## Surface and proof

- Surface: <!-- portable memory / optional adapter / engine lab -->
- Oracle: <!-- exact test, benchmark, or release gate -->
- New dependency: <!-- none, or purpose + age + license + closure impact -->

## Checklist

- [ ] `cargo fmt --all --check`
- [ ] Changed package tests and Clippy pass with warnings denied
- [ ] Portable-path changes pass `release/synapse-agent-memory/verify.sh`
- [ ] Dependency changes include license/security/14-day review
- [ ] `CHANGELOG.md` entry added when behavior or release surface changes
- [ ] Docs updated if a public API, file format, or bench number changed
- [ ] No memory DB, transcript, checkpoint, key, token, cache, corpus, or local path added
