# Qubik Patch System

This directory contains the patch scripts and configuration for the
**Create-Delight-Remake \[Qubik Friends Ver\]** modpack.

## How It Works

Every week a GitHub Actions workflow checks the upstream
[Create-Delight-Remake](https://github.com/Jasons-impart/Create-Delight-Remake)
`release` branch for new commits. When changes are detected (or when triggered
manually), the workflow:

1. **Checks out** the upstream `release` branch into a fresh workspace.
2. **Applies overlay files** — everything in `overlay/` is copied on top of the
   upstream, letting us add or replace any file (configs, KubeJS scripts, etc.).
3. **Applies mod patches** — `patches.toml` drives additions, removals, and
   substitutions of JAR files and resource packs.
4. **Builds** the patched modpack with packwiz and uploads the result as a
   GitHub Actions artifact.
5. **Commits** the new upstream SHA back to `qubik-patch/.upstream-sha` so the
   next weekly run can detect whether a rebuild is needed.

## Directory Structure

```
qubik-patch/
  apply_patches.rs    # rust-script that applies patches.toml and the overlay
  patches.toml        # patch configuration (add / remove / substitute)
  overlay/            # files to copy verbatim onto the upstream checkout
  .upstream-sha       # last processed upstream commit SHA (managed by CI)
  README.md           # this file
```

## `patches.toml` Reference

```toml
# Remove a mod by filename glob
[[mods.remove]]
pattern = "some-mod-*.jar"
reason  = "Incompatible with our KubeJS scripts"   # optional

# Add a mod by direct download URL
[[mods.add]]
url      = "https://example.com/releases/my-mod-1.0.0.jar"
filename = "my-mod-1.0.0.jar"
reason   = "Adds QoL feature X"

# Substitute a mod (remove old, download new)
[[mods.substitute]]
remove_pattern = "old-mod-*.jar"
url            = "https://example.com/releases/old-mod-2.0.0.jar"
filename       = "old-mod-2.0.0.jar"
reason         = "Upgrade to 2.0.0"

# Remove a resource pack by filename glob
[[resourcepacks.remove]]
pattern = "old-pack-*.zip"

# Add a resource pack by direct download URL
[[resourcepacks.add]]
url      = "https://example.com/releases/my-pack.zip"
filename = "my-resource-pack.zip"
```

## Overlay Files

Place any file you want to add or override in `overlay/`, mirroring the
modpack's own directory structure. For example:

```
overlay/
  config/
    some-mod-common.toml     # overrides upstream's config/some-mod-common.toml
  kubejs/
    startup_scripts/
      qubik_tweaks.js        # adds a new KubeJS startup script
```

Files are copied **before** mod patches are applied, so you can safely
reference newly added mods in your KubeJS scripts.

## Running Locally

Requires [rust-script](https://github.com/fornwall/rust-script):

```bash
cargo install rust-script
```

Apply patches to an already-checked-out upstream copy:

```bash
# From the repo root
rust-script qubik-patch/apply_patches.rs \
    /path/to/upstream-modpack \
    qubik-patch/patches.toml \
    qubik-patch/overlay
```

## CI / CD

The workflow is defined at `.github/workflows/qubik-patch-build.yml`.

- **Scheduled**: every Sunday at 02:00 UTC.
- **Manual**: Actions tab → **Build Qubik Friends Modpack** → **Run workflow**.
  Use the `Force build` checkbox to rebuild even when the upstream has not changed.

Build artifacts are uploaded under the name
`[Qubik-Friends] Create-Delight-Remake-<upstream-version>`.
