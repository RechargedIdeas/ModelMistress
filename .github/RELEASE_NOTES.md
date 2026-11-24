Custom fork of ModelMistress 0.4.2. REA and ARTEX are owner tools on the hub, and Misty can call them when that hub is started with `serve --with-rea` and `serve --with-artex`.

### Windows

- `ModelMistress_@@VER@@_x64-setup.exe` — desktop app with the hub inside (per-user, no administrator). Recommended.
- `ModelMistress_@@VER@@_x64_en-US.msi` — the same install as an MSI.
- `model-mistress-@@VER@@-windows-x64-vulkan.exe` — the hub alone (command line, API, MCP). llama.cpp is built with Vulkan.

The installers are not code-signed, so Windows SmartScreen may warn before it lets you run them.

### Linux

`model-mistress-@@VER@@-linux-src.tar.gz` is this source tree plus the pinned `engines/llama.cpp` and `engines/stable-diffusion.cpp` checkouts. Rigel is not in the archive.

```bash
tar -xzf model-mistress-@@VER@@-linux-src.tar.gz
cd model-mistress-@@VER@@-linux-src
(cd frontend && npm ci && npm run build)
cargo build --release -p model-mistress --features bundled-engine
```

Needs Rust, CMake, a C++ compiler, and the Vulkan SDK. `MM_GGML_BACKEND=cpu` builds without the SDK. Node is only for the dashboard at `/ui`.

### SHA-256

```
@@SHA@@
```
