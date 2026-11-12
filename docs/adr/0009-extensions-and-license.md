# 0009: Extensions, content and license

Status: Accepted (2026-11-12)

## Decision

### License
- Tome & Tile is licensed **AGPL-3.0-only**. Anyone who hosts a modified version must publish their changes.

### Extensions
- Third parties can extend the VTT. Extension code is a WASM module run in a sandbox: wasmtime on the server, a Worker in the browser.
- A plugin can do only what the host grants through a capability API (read the state it is shown, propose events, register abilities, draw in a panel). It has no network, file or DOM access of its own.
- Data-only extensions (rules data, monsters, maps, themes) need no code and no sandbox.

### Plugins and the AGPL
- A plugin that runs in the sandbox and uses only the published capability API is a separate work and may carry any license. This statement is to be repeated in the plugin documentation.

### Game content
- Only SRD 5.1 and SRD 5.2 content (CC-BY-4.0) ships in the repository, with the required attribution.
- All other content is imported by users from their own sources, including Creature Cauldron and Adventurer's Anvil.
- No text from any non-SRD rulebook may be committed.

## Why

- JavaScript modules in the page, as Foundry does it, can read everything the page can. Extensions run on strangers' tables, so they are sandboxed.
- The AGPL was chosen by the project owner to stop closed hosted forks.
- The SRD-only rule is the legally safe route.

## Rejected

- **MIT or Apache-2.0:** wider adoption, but allows closed forks.
- **JavaScript modules:** weak isolation.
- **Deferring the content question:** risks having to strip content later.
