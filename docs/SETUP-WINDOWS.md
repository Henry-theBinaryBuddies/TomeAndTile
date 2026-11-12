# Setting up on Windows

Run these in PowerShell. Each line is one command. Close and reopen PowerShell after the installs so the new programs are found.

## Everyone

Git (skip if `git --version` already works):

```bash
winget install --id Git.Git -e
```

Rust, which also installs `cargo`:

```bash
winget install --id Rustlang.Rustup -e
```

Rust needs the Microsoft C++ build tools. If the Rust installer asks to install them, say yes. Otherwise:

```bash
winget install --id Microsoft.VisualStudio.2022.BuildTools -e --override "--wait --passive --add Microsoft.VisualStudio.Workload.VCTools --includeRecommended"
```

Node.js LTS:

```bash
winget install --id OpenJS.NodeJS.LTS -e
```

Buf, the Protobuf tool used to check and generate the contracts:

```bash
npm.cmd install -g @bufbuild/buf
```

## If something is "not recognized"

- **`cargo`, `rustc` or `buf` not recognized right after installing:** the window was opened before the install and has an old PATH. Close every PowerShell window, and restart any app that has a built-in terminal (the Claude desktop app, your editor), then try again.
- **`npm` says "running scripts is disabled on this system":** type `npm.cmd` instead of `npm`. The same goes for `buf.cmd` if `buf` gives that error.
- **`winget` not recognized:** install Rust from https://rustup.rs and the build tools from https://visualstudio.microsoft.com/visual-cpp-build-tools/ (tick "Desktop development with C++").

Clone the repository (the URL will be confirmed once the remote exists):

```bash
git clone https://github.com/Henry-theBinaryBuddies/TomeAndTile.git
```

## Check that it works

From the repository folder. The first build downloads the toolchain named in `rust-toolchain.toml`, including the WASM target.

```bash
cargo test --workspace
```

```bash
buf lint contracts
```

```bash
bash tools/check-deps.sh
```

`bash` comes with Git for Windows. If PowerShell cannot find it, run the command from the "Git Bash" program instead.

## Only the project owner, for now

Docker Desktop, for PostgreSQL and MinIO locally:

```bash
winget install --id Docker.DockerDesktop -e
```

## Never

- Never put a password, key or token in the repository, an issue or a chat with Claude.
- Local secrets go in a `.env` file, which git ignores.
