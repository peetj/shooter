# shooter

2D top-down multiplayer shooter for the web.

## Goals (MVP)
- Rust authoritative server (tick-based)
- Rust/WASM client (canvas/WebGL later if needed)
- Binary protocol (no JSON)
- Simple top-down arena: move + aim + shoot + health/respawn

## Workspace
This repo is a Cargo workspace:
- `server/` (native)
- `client/` (wasm)
- `shared/` (types + protocol)

## Quick start (dev)
**Prereqs:** Rust stable, `wasm32-unknown-unknown` target, `trunk`

```bash
cargo build

# Terminal 1: server
cargo run -p shooter-server

# Terminal 2: client
cd client
trunk serve
```

Then open the URL trunk prints (usually http://127.0.0.1:8080).
