# @supersynergy/synapse

JS/TS bindings for the [Synapse](https://github.com/Supersynergy/synapse) memory engine via napi-rs.

## Install

```sh
bun add @supersynergy/synapse
# or
npm install @supersynergy/synapse
```

## Usage

```ts
import { Synapse } from '@supersynergy/synapse'

const s = new Synapse('./brain.synx')

const id = await s.put('doc:1', 'the quick brown fox', JSON.stringify({ tag: 'foo' }))
console.log('stored id:', id)

const hits = await s.search('quick fox', 10)
console.log(hits)

// hybrid (requires pre-computed embedding)
const embedding = new Array(384).fill(0)
const hybridHits = await s.searchHybrid('quick fox', embedding, 10)

await s.close()
```

## Platforms

| Platform | Binary |
|----------|--------|
| macOS ARM64 | `synapse-js.darwin-arm64.node` |
| macOS x64 | `synapse-js.darwin-x64.node` |
| Linux x64 | `synapse-js.x86_64-unknown-linux-gnu.node` |

## Build from source

```sh
# requires napi-rs cli
bunx @napi-rs/cli build --release --platform
```

## Publish

```sh
NPM_TOKEN=<token> bunx @napi-rs/cli pre-publish
npm publish --access public
```
