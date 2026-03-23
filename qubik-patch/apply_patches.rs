#!/usr/bin/env rust-script
//! ```cargo
//! [dependencies]
//! serde = { version = "1", features = ["derive"] }
//! toml = "0.8"
//! reqwest = { version = "0.12", features = ["blocking"] }
//! glob = "0.3"
//! anyhow = "1"
//! ```

//! Apply Qubik-Friends patches to a modpack directory.
//!
//! Usage:
//!   apply_patches.rs <modpack-dir> [patches.toml] [overlay-dir] [assets-dir]
//!
//! Arguments:
//!   modpack-dir   Path to the modpack directory to patch (required)
//!   patches.toml  Path to the patch config file (default: patches.toml in CWD)
//!   overlay-dir   Path to the overlay directory (default: overlay/ in CWD)
//!   assets-dir    Path to the assets directory (default: assets/ in CWD)

use anyhow::{bail, Context, Result};
use serde::Deserialize;
use std::{
    fs,
    path::{Path, PathBuf},
};

// ---------------------------------------------------------------------------
// Schema
// ---------------------------------------------------------------------------

#[derive(Deserialize, Default)]
#[serde(default)]
struct Patches {
    mods: ModPatches,
    resourcepacks: ResourcePackPatches,
    assets: Vec<AssetEntry>,
}

#[derive(Deserialize)]
struct AssetEntry {
    /// Path of the source directory, relative to the assets-dir argument.
    src: String,
    /// Destination path relative to the modpack directory.
    dest: String,
    #[serde(default)]
    mode: AssetMode,
    #[serde(default)]
    reason: String,
}

#[derive(Deserialize, Default, PartialEq)]
#[serde(rename_all = "lowercase")]
enum AssetMode {
    /// Copy files into dest, leaving any existing files that are not overwritten.
    #[default]
    Merge,
    /// Delete dest entirely before copying, so the result contains only the
    /// files from src.
    Replace,
}

#[derive(Deserialize, Default)]
#[serde(default)]
struct ModPatches {
    add: Vec<AddEntry>,
    remove: Vec<RemoveEntry>,
    substitute: Vec<SubstituteEntry>,
}

#[derive(Deserialize, Default)]
#[serde(default)]
struct ResourcePackPatches {
    add: Vec<AddEntry>,
    remove: Vec<RemoveEntry>,
}

#[derive(Deserialize)]
struct AddEntry {
    url: String,
    filename: String,
    #[serde(default)]
    reason: String,
}

#[derive(Deserialize)]
struct RemoveEntry {
    pattern: String,
    #[serde(default)]
    reason: String,
}

#[derive(Deserialize)]
struct SubstituteEntry {
    remove_pattern: String,
    url: String,
    filename: String,
    #[serde(default)]
    reason: String,
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

fn download_file(url: &str, dest: &Path) -> Result<()> {
    println!("  Downloading {} -> {}", url, dest.display());
    let response = reqwest::blocking::get(url)
        .with_context(|| format!("HTTP request failed for {url}"))?;
    if !response.status().is_success() {
        bail!("HTTP {} for {url}", response.status());
    }
    let bytes = response
        .bytes()
        .with_context(|| format!("Failed to read response body from {url}"))?;
    if let Some(parent) = dest.parent() {
        fs::create_dir_all(parent)?;
    }
    fs::write(dest, &bytes)
        .with_context(|| format!("Failed to write {}", dest.display()))?;
    println!("  -> OK ({} bytes)", bytes.len());
    Ok(())
}

/// Remove all files in `dir` matching `pattern` (glob against filename only).
/// Returns the number of files removed. Warns if nothing matched.
fn remove_by_pattern(dir: &Path, pattern: &str) -> Result<usize> {
    let full_pattern = dir.join(pattern).to_string_lossy().into_owned();
    let mut count = 0;
    for entry in glob::glob(&full_pattern).context("Invalid glob pattern")? {
        let path = entry.context("Glob iteration error")?;
        println!("  Removing {}", path.display());
        fs::remove_file(&path)
            .with_context(|| format!("Failed to remove {}", path.display()))?;
        count += 1;
    }
    if count == 0 {
        eprintln!("  Warning: no files matched pattern '{pattern}' in {}", dir.display());
    }
    Ok(count)
}

/// Recursively collect all files under `dir`.
fn collect_files(dir: &Path) -> Result<Vec<PathBuf>> {
    let mut result = Vec::new();
    if !dir.is_dir() {
        return Ok(result);
    }
    for entry in
        fs::read_dir(dir).with_context(|| format!("Cannot read directory {}", dir.display()))?
    {
        let path = entry?.path();
        if path.is_dir() {
            result.extend(collect_files(&path)?);
        } else {
            result.push(path);
        }
    }
    Ok(result)
}

/// Copy all files from `src` into `dst`, preserving relative structure.
/// Skips `.gitkeep` files.
fn copy_overlay(src: &Path, dst: &Path) -> Result<usize> {
    let mut count = 0;
    for file in collect_files(src)? {
        if file.file_name().map(|n| n == ".gitkeep").unwrap_or(false) {
            continue;
        }
        let rel = file
            .strip_prefix(src)
            .with_context(|| format!("strip_prefix failed for {}", file.display()))?;
        let dest = dst.join(rel);
        if let Some(parent) = dest.parent() {
            fs::create_dir_all(parent)?;
        }
        println!("  Overlaying {}", rel.display());
        fs::copy(&file, &dest)
            .with_context(|| format!("Failed to copy {} -> {}", file.display(), dest.display()))?;
        count += 1;
    }
    Ok(count)
}

/// Copy an asset source directory into `dest` inside the modpack.
///
/// * `Merge`   — copy files on top of any existing content.
/// * `Replace` — delete `dest` first, then copy, so only files from `src` remain.
fn apply_assets(src: &Path, dest: &Path, mode: &AssetMode) -> Result<usize> {
    if !src.is_dir() {
        bail!("Assets source directory not found: {}", src.display());
    }
    if *mode == AssetMode::Replace && dest.exists() {
        println!("  Replacing (removing existing) {}", dest.display());
        fs::remove_dir_all(dest)
            .with_context(|| format!("Failed to remove {}", dest.display()))?;
    }
    fs::create_dir_all(dest)
        .with_context(|| format!("Failed to create {}", dest.display()))?;
    copy_overlay(src, dest)
}

fn reason_suffix(reason: &str) -> String {
    if reason.is_empty() {
        String::new()
    } else {
        format!(" ({})", reason)
    }
}

// ---------------------------------------------------------------------------
// Main
// ---------------------------------------------------------------------------

fn main() -> Result<()> {
    let args: Vec<String> = std::env::args().collect();

    if args.len() < 2 {
        eprintln!("Usage: {} <modpack-dir> [patches.toml] [overlay-dir] [assets-dir]", args[0]);
        std::process::exit(1);
    }

    let modpack_dir = PathBuf::from(&args[1]);
    let patches_file = args
        .get(2)
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("patches.toml"));
    let overlay_dir = args.get(3).map(PathBuf::from);
    let assets_dir = args.get(4).map(PathBuf::from);

    if !modpack_dir.is_dir() {
        bail!("Modpack directory not found: {}", modpack_dir.display());
    }

    // --- Apply overlay -----------------------------------------------------------
    if let Some(ref overlay) = overlay_dir {
        if overlay.is_dir() {
            let all_files = collect_files(overlay)?;
            let meaningful: Vec<_> = all_files
                .iter()
                .filter(|p| p.file_name().map(|n| n != ".gitkeep").unwrap_or(true))
                .collect();

            if meaningful.is_empty() {
                println!("=== Overlay directory is empty — skipping ===");
            } else {
                println!("=== Applying overlay from {} ===", overlay.display());
                let count = copy_overlay(overlay, &modpack_dir)?;
                println!("  Overlaid {count} file(s).");
            }
        } else {
            eprintln!(
                "Warning: overlay directory '{}' not found — skipping",
                overlay.display()
            );
        }
    }

    // --- Load patches.toml -------------------------------------------------------
    if !patches_file.exists() {
        println!(
            "No patches file found at {} — skipping asset/mod/resourcepack patches.",
            patches_file.display()
        );
        println!("Done.");
        return Ok(());
    }

    let content = fs::read_to_string(&patches_file)
        .with_context(|| format!("Failed to read {}", patches_file.display()))?;
    let patches: Patches =
        toml::from_str(&content).context("Failed to parse patches.toml")?;

    let mods_dir = modpack_dir.join("mods");
    let resourcepacks_dir = modpack_dir.join("resourcepacks");

    // --- Asset patches -----------------------------------------------------------
    if !patches.assets.is_empty() {
        match assets_dir {
            None => {
                eprintln!("Warning: [[assets]] entries defined but no assets-dir argument provided — skipping assets");
            }
            Some(ref adir) if !adir.is_dir() => {
                eprintln!(
                    "Warning: assets directory '{}' not found — skipping assets",
                    adir.display()
                );
            }
            Some(ref adir) => {
                println!("\n=== Applying asset patches ===");
                for entry in &patches.assets {
                    let src = adir.join(&entry.src);
                    let dest = modpack_dir.join(&entry.dest);
                    let mode_label = if entry.mode == AssetMode::Replace { "replace" } else { "merge" };
                    println!(
                        "[{}] {} -> {}{}",
                        mode_label,
                        entry.src,
                        entry.dest,
                        reason_suffix(&entry.reason)
                    );
                    let count = apply_assets(&src, &dest, &entry.mode)?;
                    println!("  Copied {count} file(s).");
                }
            }
        }
    }

    // --- Mod patches -------------------------------------------------------------
    let has_mod_patches = !patches.mods.remove.is_empty()
        || !patches.mods.add.is_empty()
        || !patches.mods.substitute.is_empty();

    if has_mod_patches {
        if !mods_dir.is_dir() {
            bail!(
                "mods/ directory not found in modpack: {}",
                mods_dir.display()
            );
        }

        println!("\n=== Applying mod patches ===");

        // Removals
        for entry in &patches.mods.remove {
            println!(
                "Removing mods matching '{}'{}",
                entry.pattern,
                reason_suffix(&entry.reason)
            );
            remove_by_pattern(&mods_dir, &entry.pattern)?;
        }

        // Substitutions (remove old version, download new)
        for entry in &patches.mods.substitute {
            println!(
                "Substituting '{}' -> '{}'{}",
                entry.remove_pattern,
                entry.filename,
                reason_suffix(&entry.reason)
            );
            remove_by_pattern(&mods_dir, &entry.remove_pattern)?;
            download_file(&entry.url, &mods_dir.join(&entry.filename))?;
        }

        // Additions
        for entry in &patches.mods.add {
            println!(
                "Adding mod '{}'{}",
                entry.filename,
                reason_suffix(&entry.reason)
            );
            download_file(&entry.url, &mods_dir.join(&entry.filename))?;
        }
    }

    // --- Resource pack patches ---------------------------------------------------
    let has_rp_patches =
        !patches.resourcepacks.remove.is_empty() || !patches.resourcepacks.add.is_empty();

    if has_rp_patches {
        println!("\n=== Applying resource pack patches ===");

        // Removals
        for entry in &patches.resourcepacks.remove {
            println!(
                "Removing resource packs matching '{}'{}",
                entry.pattern,
                reason_suffix(&entry.reason)
            );
            remove_by_pattern(&resourcepacks_dir, &entry.pattern)?;
        }

        // Additions
        for entry in &patches.resourcepacks.add {
            println!(
                "Adding resource pack '{}'{}",
                entry.filename,
                reason_suffix(&entry.reason)
            );
            if !resourcepacks_dir.exists() {
                fs::create_dir_all(&resourcepacks_dir)?;
            }
            download_file(&entry.url, &resourcepacks_dir.join(&entry.filename))?;
        }
    }

    let has_patches = !patches.assets.is_empty() || has_mod_patches || has_rp_patches;
    if !has_patches {
        println!("\nNo patches defined in patches.toml — nothing to do.");
    } else {
        println!("\nAll patches applied successfully.");
    }

    Ok(())
}
