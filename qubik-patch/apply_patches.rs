#!/usr/bin/env rust-script
//! ```cargo
//! [dependencies]
//! serde = { version = "1", features = ["derive"] }
//! toml = "0.8"
//! reqwest = { version = "0.12", features = ["blocking"] }
//! glob = "0.3"
//! anyhow = "1"
//! ferinth = "2"
//! furse = "1"
//! tokio = { version = "1", features = ["rt-multi-thread"] }
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
//!
//! Environment variables:
//!   CURSEFORGE_API_KEY  CurseForge API key (required when any entry uses `curseforge`)

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
    /// The Minecraft game version this patch targets (e.g. `"1.20.1"`).
    /// When set and a mod/resource-pack entry has no explicit `version`,
    /// the resolver fetches the latest file that supports this game version
    /// instead of the globally latest file.
    game_version: Option<String>,
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

/// A file to download. Exactly one of `url`, `modrinth`, or `curseforge` must be set.
#[derive(Deserialize)]
struct AddEntry {
    /// Direct download URL. Required if neither `modrinth` nor `curseforge` is set.
    url: Option<String>,

    /// Modrinth project ID or slug.
    modrinth: Option<String>,

    /// CurseForge project ID (integer).
    curseforge: Option<i32>,

    /// Version to download.
    /// - For `modrinth`: first tries an exact `version_number` match (e.g. `"mc1.20.1-0.5.8"`);
    ///   if nothing matches, treats the value as a Minecraft game version and picks the newest
    ///   mod version that lists it in its `game_versions` (e.g. `"1.20.1"`).
    /// - For `curseforge`: matched as a substring of the file's `display_name`.
    /// - Omit (or set to `"latest"`) to use the newest available version.
    version: Option<String>,

    /// Destination filename inside the target directory.
    /// Required when using `url`. Optional for `modrinth`/`curseforge` (auto-derived from API).
    filename: Option<String>,

    /// If true, the mod is client-only and its identifier will be added to .clientonlymodlist.
    /// For modrinth entries the modrinth slug is used; for curseforge/url the resolved filename
    /// without the .jar extension is used.
    #[serde(default)]
    clientonly: bool,

    #[serde(default)]
    reason: String,
}

#[derive(Deserialize)]
struct RemoveEntry {
    pattern: String,
    #[serde(default)]
    reason: String,
}

/// Remove a file matching `remove_pattern` and replace it with a new download.
/// Exactly one of `url`, `modrinth`, or `curseforge` must be set.
#[derive(Deserialize)]
struct SubstituteEntry {
    remove_pattern: String,

    /// Direct download URL. Required if neither `modrinth` nor `curseforge` is set.
    url: Option<String>,

    /// Modrinth project ID or slug.
    modrinth: Option<String>,

    /// CurseForge project ID (integer).
    curseforge: Option<i32>,

    /// Version to download (see `AddEntry::version` for semantics).
    /// For modrinth: exact version_number OR Minecraft game version (e.g. "1.20.1").
    version: Option<String>,

    /// Destination filename. Required when using `url`. Optional for `modrinth`/`curseforge`.
    filename: Option<String>,

    /// If true, the mod is client-only and its identifier will be added to .clientonlymodlist.
    /// For modrinth entries the modrinth slug is used; for curseforge/url the resolved filename
    /// without the .jar extension is used.
    #[serde(default)]
    clientonly: bool,

    #[serde(default)]
    reason: String,
}

// ---------------------------------------------------------------------------
// Source resolution
// ---------------------------------------------------------------------------

/// Given the source fields from an entry, resolve to `(download_url, filename)`.
fn resolve_source(
    rt: &tokio::runtime::Runtime,
    url: Option<&str>,
    modrinth: Option<&str>,
    curseforge: Option<i32>,
    version: Option<&str>,
    filename: Option<&str>,
    cf_api_key: &Option<String>,
    game_version: Option<&str>,
) -> Result<(String, String)> {
    let use_latest = version.map(|v| v.eq_ignore_ascii_case("latest")).unwrap_or(true);
    let version_req = if use_latest { None } else { version };

    match (url, modrinth, curseforge) {
        (Some(u), None, None) => {
            let fname = filename.ok_or_else(|| {
                anyhow::anyhow!("`filename` is required when using `url`")
            })?;
            Ok((u.to_owned(), fname.to_owned()))
        }
        (None, Some(project), None) => {
            let (dl_url, api_filename) =
                resolve_modrinth(rt, project, version_req, game_version)
                    .with_context(|| format!("Failed to resolve Modrinth project '{project}'"))?;
            // User-supplied filename overrides the API-derived one.
            let fname = filename.map(str::to_owned).unwrap_or(api_filename);
            Ok((dl_url, fname))
        }
        (None, None, Some(project_id)) => {
            let key = cf_api_key.as_deref().ok_or_else(|| {
                anyhow::anyhow!(
                    "CurseForge entry found but CURSEFORGE_API_KEY is not set. \
                     Export the variable before running this script."
                )
            })?;
            let (dl_url, api_filename) =
                resolve_curseforge(rt, project_id, key, version_req, game_version)
                    .with_context(|| {
                        format!("Failed to resolve CurseForge project id {project_id}")
                    })?;
            let fname = filename.map(str::to_owned).unwrap_or(api_filename);
            Ok((dl_url, fname))
        }
        _ => bail!(
            "Specify exactly one of `url`, `modrinth`, or `curseforge` per entry"
        ),
    }
}

/// Resolve a Modrinth project slug from either a project ID or a slug.
/// Uses the Modrinth REST API directly so that both forms (e.g. `"Es5v4eyq"`
/// and `"sodium"`) always produce the canonical slug stored in the project record.
fn resolve_modrinth_slug(project_id_or_slug: &str) -> Result<String> {
    #[derive(serde::Deserialize)]
    struct ProjectInfo {
        slug: String,
    }
    let url = format!(
        "https://api.modrinth.com/v2/project/{}",
        project_id_or_slug
    );
    let response = reqwest::blocking::get(&url)
        .with_context(|| format!("HTTP request failed for {url}"))?;
    if !response.status().is_success() {
        bail!(
            "HTTP {} while fetching Modrinth project '{project_id_or_slug}'",
            response.status()
        );
    }
    let info: ProjectInfo = response
        .json()
        .with_context(|| format!("Failed to parse project info for '{project_id_or_slug}'"))?;
    Ok(info.slug)
}

/// Resolve a Modrinth project to `(download_url, filename)` using ferinth.
fn resolve_modrinth(
    rt: &tokio::runtime::Runtime,
    project_id: &str,
    version: Option<&str>,
    game_version: Option<&str>,
) -> Result<(String, String)> {
    rt.block_on(async {
        let ferinth = ferinth::Ferinth::<()>::new(
            "apply_patches",
            Some(env!("CARGO_PKG_VERSION")),
            None,
        );

        let versions = ferinth
            .version_list(project_id)
            .await
            .context("Failed to list Modrinth versions")?;

        if versions.is_empty() {
            bail!("No versions found for Modrinth project '{project_id}'");
        }

        let chosen = match version {
            None => {
                // No explicit version requested — use the newest release that
                // supports `game_version` (when set) or globally newest.
                if let Some(gv) = game_version {
                    versions
                        .into_iter()
                        .find(|ver| {
                            ver.game_versions.iter().any(|v| v.eq_ignore_ascii_case(gv))
                        })
                        .ok_or_else(|| {
                            anyhow::anyhow!(
                                "No version for game version '{gv}' found for \
                                 Modrinth project '{project_id}'"
                            )
                        })?
                } else {
                    versions.into_iter().next().unwrap()
                }
            }
            Some(v) => versions
                .into_iter()
                // Match against version_number (e.g. "mc1.20.1-0.5.8") first;
                // if nothing matches, treat v as a Minecraft game version and
                // pick the newest version that supports it (e.g. "1.20.1").
                .find(|ver| {
                    ver.version_number == v
                        || ver.game_versions.iter().any(|gv| gv.eq_ignore_ascii_case(v))
                })
                .ok_or_else(|| {
                    anyhow::anyhow!(
                        "No version matching '{v}' (as version_number or game version) \
                         found for Modrinth project '{project_id}'"
                    )
                })?,
        };

        println!(
            "  Modrinth: {} version {}",
            project_id, chosen.version_number
        );

        let file = chosen
            .files
            .iter()
            .find(|f| f.primary)
            .or_else(|| chosen.files.first())
            .ok_or_else(|| anyhow::anyhow!("Version has no files"))?;

        Ok((file.url.to_string(), file.filename.clone()))
    })
}

/// Resolve a CurseForge project to `(download_url, filename)` using furse.
fn resolve_curseforge(
    rt: &tokio::runtime::Runtime,
    project_id: i32,
    api_key: &str,
    version: Option<&str>,
    game_version: Option<&str>,
) -> Result<(String, String)> {
    rt.block_on(async {
        let furse = furse::Furse::new(api_key);

        let mut files = furse
            .get_mod_files(project_id)
            .await
            .context("Failed to list CurseForge files")?;

        if files.is_empty() {
            bail!("No files found for CurseForge project id {project_id}");
        }

        // Sort by id descending so the newest file is first.
        files.sort_by(|a, b| b.id.cmp(&a.id));

        let chosen = match version {
            None => {
                // No explicit version requested — use the newest file that
                // supports `game_version` (when set) or globally newest.
                if let Some(gv) = game_version {
                    files
                        .into_iter()
                        .find(|f| {
                            f.game_versions
                                .iter()
                                .any(|v| v.eq_ignore_ascii_case(gv))
                        })
                        .ok_or_else(|| {
                            anyhow::anyhow!(
                                "No file for game version '{gv}' found for \
                                 CurseForge project id {project_id}"
                            )
                        })?
                } else {
                    files.into_iter().next().unwrap()
                }
            }
            Some(v) => files
                .into_iter()
                .find(|f| f.display_name.contains(v) || f.file_name.contains(v))
                .ok_or_else(|| {
                    anyhow::anyhow!(
                        "Version '{v}' not found for CurseForge project id {project_id}"
                    )
                })?,
        };

        println!(
            "  CurseForge: project {} file '{}' (id {})",
            project_id, chosen.display_name, chosen.id
        );

        let dl_url = chosen.download_url.ok_or_else(|| {
            anyhow::anyhow!(
                "CurseForge file '{}' has no download URL \
                 (the mod author may have disabled third-party distribution)",
                chosen.display_name
            )
        })?;

        Ok((dl_url.to_string(), chosen.file_name))
    })
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
// Client-only list helpers
// ---------------------------------------------------------------------------

/// Derive the identifier used in `.clientonlymodlist` for an entry.
/// For modrinth entries the canonical project slug is resolved from the API so
/// that both IDs (e.g. `"Es5v4eyq"`) and slugs (e.g. `"sodium"`) produce the
/// same human-readable name.  For curseforge/url entries the resolved filename
/// without the `.jar` extension is used.
fn clientonly_id(modrinth: Option<&str>, filename: &str) -> Result<String> {
    if let Some(id_or_slug) = modrinth {
        let slug = resolve_modrinth_slug(id_or_slug).unwrap_or_else(|err| {
            eprintln!(
                "  Warning: could not resolve Modrinth slug for '{}': {err:#}; \
                 using raw value as identifier",
                id_or_slug
            );
            id_or_slug.to_owned()
        });
        Ok(slug)
    } else {
        Ok(filename.strip_suffix(".jar").unwrap_or(filename).to_owned())
    }
}

/// Append any `new_ids` that are not already present in `list_path`.
/// The file is created if it does not exist. New entries are appended in
/// sorted order after the existing content.
fn update_clientonly_list(list_path: &Path, new_ids: &[String]) -> Result<()> {
    if new_ids.is_empty() {
        return Ok(());
    }
    let existing = if list_path.exists() {
        fs::read_to_string(list_path)
            .with_context(|| format!("Failed to read {}", list_path.display()))?
    } else {
        String::new()
    };
    let existing_set: std::collections::HashSet<&str> = existing.lines().collect();
    let mut to_add: Vec<&str> = new_ids
        .iter()
        .map(String::as_str)
        .filter(|id| !existing_set.contains(id))
        .collect();
    if to_add.is_empty() {
        return Ok(());
    }
    to_add.sort_unstable();
    let mut content = existing;
    if !content.is_empty() && !content.ends_with('\n') {
        content.push('\n');
    }
    for id in &to_add {
        content.push_str(id);
        content.push('\n');
    }
    fs::write(list_path, &content)
        .with_context(|| format!("Failed to write {}", list_path.display()))?;
    println!(
        "  Updated {}: added {}",
        list_path.display(),
        to_add.join(", ")
    );
    Ok(())
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

    // Read CurseForge API key once; validated lazily when a CF entry is encountered.
    let cf_api_key: Option<String> = std::env::var("CURSEFORGE_API_KEY").ok();

    // Tokio runtime for async API calls (Modrinth / CurseForge). Created lazily
    // only when the patches file exists and contains modrinth/curseforge entries,
    // but it's cheap to create upfront.
    let rt = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .context("Failed to build Tokio runtime")?;

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

    // Path to the client-only mod list, resolved relative to the patches file.
    let clientonly_list_path = {
        let parent = patches_file.parent().filter(|p| !p.as_os_str().is_empty());
        parent.unwrap_or_else(|| Path::new(".")).join(".clientonlymodlist")
    };

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

        let mut clientonly_ids: Vec<String> = Vec::new();

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
                entry.filename.as_deref().unwrap_or("<auto>"),
                reason_suffix(&entry.reason)
            );
            remove_by_pattern(&mods_dir, &entry.remove_pattern)?;
            let (url, filename) = resolve_source(
                &rt,
                entry.url.as_deref(),
                entry.modrinth.as_deref(),
                entry.curseforge,
                entry.version.as_deref(),
                entry.filename.as_deref(),
                &cf_api_key,
                patches.game_version.as_deref(),
            )?;
            download_file(&url, &mods_dir.join(&filename))?;
            if entry.clientonly {
                clientonly_ids.push(clientonly_id(entry.modrinth.as_deref(), &filename)?);
            }
        }

        // Additions
        for entry in &patches.mods.add {
            let (url, filename) = resolve_source(
                &rt,
                entry.url.as_deref(),
                entry.modrinth.as_deref(),
                entry.curseforge,
                entry.version.as_deref(),
                entry.filename.as_deref(),
                &cf_api_key,
                patches.game_version.as_deref(),
            )?;
            println!(
                "Adding mod '{}'{}",
                filename,
                reason_suffix(&entry.reason)
            );
            download_file(&url, &mods_dir.join(&filename))?;
            if entry.clientonly {
                clientonly_ids.push(clientonly_id(entry.modrinth.as_deref(), &filename)?);
            }
        }

        if !clientonly_ids.is_empty() {
            println!("\n=== Updating client-only mod list ===");
            update_clientonly_list(&clientonly_list_path, &clientonly_ids)?;
            // Merge the same IDs into the modpack's copy, preserving any
            // entries that were already there.
            let modpack_list_path = modpack_dir.join(".clientonlymodlist");
            update_clientonly_list(&modpack_list_path, &clientonly_ids)?;
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
            let (url, filename) = resolve_source(
                &rt,
                entry.url.as_deref(),
                entry.modrinth.as_deref(),
                entry.curseforge,
                entry.version.as_deref(),
                entry.filename.as_deref(),
                &cf_api_key,
                patches.game_version.as_deref(),
            )?;
            println!(
                "Adding resource pack '{}'{}",
                filename,
                reason_suffix(&entry.reason)
            );
            if !resourcepacks_dir.exists() {
                fs::create_dir_all(&resourcepacks_dir)?;
            }
            download_file(&url, &resourcepacks_dir.join(&filename))?;
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
