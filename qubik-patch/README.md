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
3. **Applies asset patches** — `[[assets]]` entries in `patches.toml` copy
   subdirectories from `assets/` to designated locations in the modpack, with a
   choice of **merge** (add on top) or **replace** (wipe destination first).
4. **Applies mod patches** — `patches.toml` drives additions, removals, and
   substitutions of JAR files and resource packs.
5. **Builds** the patched modpack with packwiz and uploads the result as a
   GitHub Actions artifact.
6. **Commits** the new upstream SHA back to `qubik-patch/.upstream-sha` so the
   next weekly run can detect whether a rebuild is needed.

## Directory Structure

```
qubik-patch/
  apply_patches.rs    # rust-script that applies patches.toml, the overlay, and assets
  compare_modlist.rs  # standalone Rust checker for two modpack directories
  patches.toml        # patch configuration (add / remove / substitute / assets)
  overlay/            # files to copy verbatim onto the upstream checkout
  assets/             # named subdirectories copied to designated modpack locations
  .upstream-sha       # last processed upstream commit SHA (managed by CI)
  README.md           # this file
```

## `patches.toml` Reference

```toml
# Copy an asset directory into the modpack (merge — keeps existing files)
[[assets]]
src    = "my-textures"                    # subdirectory inside assets/
dest   = "resourcepacks/MyPack/assets"    # path relative to the modpack root
mode   = "merge"                          # optional, "merge" is the default
reason = "Our custom textures for MyPack" # optional

# Copy an asset directory, wiping the destination first (replace)
[[assets]]
src    = "generated-configs"
dest   = "config/some-mod"
mode   = "replace"
reason = "Fully managed config — no upstream leftovers wanted"

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

Files are copied **before** asset and mod patches are applied, so you can safely
reference newly added mods in your KubeJS scripts.

## Asset Files

Place resource directories in `assets/`, one subdirectory per logical group.
Each group is referenced by an `[[assets]]` entry in `patches.toml` and copied
to the designated destination inside the modpack.

```
assets/
  my-textures/            # src = "my-textures" in patches.toml
    block/
      custom_stone.png
    item/
      custom_sword.png
```

Two copy modes are available:

| Mode      | Behaviour |
|-----------|-----------|
| `merge`   | Copy files into `dest`, leaving any existing files that are not overwritten untouched. |
| `replace` | Delete `dest` entirely first, then copy — the result contains **only** the files from `src`. |

Asset patches are applied **after** the overlay but **before** mod patches.

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
    qubik-patch/overlay \
    qubik-patch/assets
```

Compare an upstream modpack directory against a patched modpack directory. Each
directory must contain a generated CurseForge-style `manifest.json` and
`modlist.html`:

```bash
rustc qubik-patch/compare_modlist.rs -O -o /tmp/compare_modlist
/tmp/compare_modlist /path/to/upstream-modpack /path/to/patched-modpack
```

The comparison reads only `files[].projectID` from each manifest and numeric
`/projects/<id>` links from each HTML mod list. It reports additions and
removals for manifests and mod lists independently, and exits non-zero if
either pair differs, any input contains duplicate project IDs, or a required
file is unreadable or malformed.

## CI / CD

The workflow is defined at `.github/workflows/qubik-patch-build.yml`.

- **Scheduled**: every Sunday at 02:00 UTC.
- **Manual**: Actions tab → **Build Qubik Friends Modpack** → **Run workflow**.
  Use the `Force build` checkbox to rebuild even when the upstream has not changed.

Build artifacts are uploaded under the name
`[Qubik-Friends] Create-Delight-Remake-<upstream-version>`.
