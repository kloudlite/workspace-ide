/// Nix package manager — host daemon, shared store, per-HOME profiles
/// Supports ws.yaml + ws.lock for reproducible workspaces

use std::collections::HashMap;
use std::path::Path;
use std::process::Command;

const NIX_BIN: &str = "/nix/var/nix/profiles/default/bin/nix";
const NIX_PROFILE_BIN: &str = "/nix/var/nix/profiles/default/bin";

/// Add nix binaries to PATH at startup
pub fn setup_env() {
    if let Ok(current) = std::env::var("PATH") {
        if !current.contains(NIX_PROFILE_BIN) {
            let new_path = format!("{}:{}", NIX_PROFILE_BIN, current);
            std::env::set_var("PATH", &new_path);
        }
    }
}

fn nix_cmd() -> Result<Command, String> {
    let nix = find_nix().ok_or_else(|| "Nix not found. Mount /nix from host.".to_string())?;
    let mut cmd = Command::new(&nix);
    cmd.env("NIX_REMOTE", "daemon");
    cmd.arg("--extra-experimental-features");
    cmd.arg("nix-command flakes");
    Ok(cmd)
}

fn find_nix() -> Option<String> {
    let paths = [NIX_BIN, "/usr/bin/nix", "/usr/local/bin/nix"];
    for p in &paths {
        if Path::new(p).exists() {
            return Some(p.to_string());
        }
    }
    None
}

/// Install a package, pinning to locked nixpkgs revision if ws.lock exists
pub fn install(package: &str) -> Result<String, String> {
    let flake = resolve_flake();
    let attr = format!("{}#{}", flake, package);
    eprintln!("ws: nix installing {} from {}...", package, flake);
    let output = nix_cmd()?
        .args(["profile", "install", &attr])
        .output()
        .map_err(|e| format!("nix install: {}", e))?;
    if output.status.success() {
        eprintln!("ws: package {} installed", package);
        let _ = write_workspace_files();
        Ok("done".into())
    } else {
        Err(String::from_utf8_lossy(&output.stderr).trim().to_string())
    }
}

/// Resolve the nixpkgs flake URL — use pinned revision from ws.lock if available
fn resolve_flake() -> String {
    if let Ok(content) = std::fs::read_to_string("ws.lock") {
        for line in content.lines() {
            let t = line.trim();
            if let Some(rev) = t.strip_prefix("nixpkgs_revision: \"") {
                if let Some(rev) = rev.strip_suffix("\"") {
                    if !rev.is_empty() {
                        return format!("github:NixOS/nixpkgs/{}", rev);
                    }
                }
            }
        }
    }
    "nixpkgs".to_string()
}

/// Remove a package and update ws.yaml + ws.lock
/// Sync ws.yaml + ws.lock from current installed state
pub fn sync() -> Result<String, String> {
    write_workspace_files()?;
    Ok("ws.yaml + ws.lock updated".into())
}

pub fn remove(package: &str) -> Result<String, String> {
    let output = nix_cmd()?
        .args(["profile", "remove", package])
        .output()
        .map_err(|e| format!("nix remove: {}", e))?;
    if output.status.success() {
        let _ = write_workspace_files();
        Ok("removed".into())
    } else {
        Err(String::from_utf8_lossy(&output.stderr).trim().to_string())
    }
}

pub fn search(query: &str) -> Result<Vec<String>, String> {
    let output = nix_cmd()?
        .args(["search", "nixpkgs", query])
        .output()
        .map_err(|e| format!("nix search: {}", e))?;
    Ok(String::from_utf8_lossy(&output.stdout).lines().map(|l| l.to_string()).collect())
}

/// List installed packages (names only)
pub fn list() -> Result<Vec<String>, String> {
    get_installed().map(|m| m.into_keys().collect())
}

/// Detailed list with package info (name, version, store path)
#[derive(serde::Serialize)]
pub struct PkgInfo {
    pub name: String,
    pub version: String,
    pub store_path: String,
}

pub fn list_detailed() -> Result<Vec<PkgInfo>, String> {
    let output = nix_cmd()?
        .args(["profile", "list", "--json"])
        .output()
        .map_err(|e| format!("nix list: {}", e))?;
    let stdout = String::from_utf8_lossy(&output.stdout);
    match serde_json::from_str::<serde_json::Value>(&stdout) {
        Ok(val) => {
            let elements = val.get("elements").and_then(|e| e.as_object()).ok_or("bad format")?;
            let mut result = Vec::new();
            for (name, info) in elements {
                let store_path = info.get("storePaths")
                    .and_then(|p| p.as_array())
                    .and_then(|a| a.first())
                    .and_then(|v| v.as_str())
                    .unwrap_or("")
                    .to_string();
                let version = extract_version(&store_path);
                result.push(PkgInfo { name: name.clone(), version, store_path });
            }
            Ok(result)
        }
        Err(e) => Err(format!("parse error: {}", e)),
    }
}

fn get_installed() -> Result<HashMap<String, String>, String> {
    let output = nix_cmd()?
        .args(["profile", "list", "--json"])
        .output()
        .map_err(|e| format!("nix list: {}", e))?;
    let stdout = String::from_utf8_lossy(&output.stdout);
    match serde_json::from_str::<serde_json::Value>(&stdout) {
        Ok(val) => {
            let mut map = HashMap::new();
            if let Some(elements) = val.get("elements").and_then(|e| e.as_object()) {
                for (name, info) in elements {
                    let store = info.get("storePaths")
                        .and_then(|p| p.as_array())
                        .and_then(|a| a.first())
                        .and_then(|v| v.as_str())
                        .unwrap_or("");
                    map.insert(name.clone(), store.to_string());
                }
            }
            Ok(map)
        }
        Err(_) => Ok(HashMap::new()),
    }
}

/// Extract version from a Nix store path like /nix/store/hash-go-1.26.4
fn extract_version(store_path: &str) -> String {
    let name = store_path.rsplit('/').next().unwrap_or("");
    // Format: hash-name-version  e.g. "gb0njhqswlc5n127ikgyikvq39r40l6f-go-1.26.4"
    let parts: Vec<&str> = name.splitn(2, '-').collect();
    if parts.len() < 2 { return "".into(); }
    let rest = parts[1]; // "go-1.26.4"
    let dash = rest.find('-').unwrap_or(rest.len());
    let version = &rest[dash + 1..];
    version.to_string()
}

/// Read ws.yaml and install missing packages
pub fn apply_yaml() -> Result<String, String> {
    let content = std::fs::read_to_string("ws.yaml")
        .map_err(|e| format!("cannot read ws.yaml: {}", e))?;
    let mut desired = Vec::new();
    for line in content.lines() {
        let t = line.trim();
        if let Some(name) = t.strip_prefix("- ") {
            if !name.is_empty() { desired.push(name.to_string()); }
        }
    }
    let current = list().unwrap_or_default();
    let mut installed = Vec::new();
    for pkg in &desired {
        if current.contains(pkg) {
            eprintln!("ws: {} already installed", pkg);
        } else {
            match install(pkg) {
                Ok(m) => installed.push(format!("  {}: {}", pkg, m)),
                Err(e) => eprintln!("ws: {} install failed: {}", pkg, e),
            }
        }
    }
    if installed.is_empty() { Ok("all packages already installed".into()) }
    else { Ok(format!("installed:\n{}", installed.join("\n"))) }
}

/// Write ws.yaml (user manifest) and ws.lock (revision pin)
fn write_workspace_files() -> Result<(), String> {
    let pkgs = get_installed()?;
    let mut yaml = String::new();
    let mut lock_pkgs = Vec::new();
    let mut yaml_pkgs = Vec::new();

    for (name, store_path) in &pkgs {
        if name == "nix" || name == "nix-manual" || name == "nss-cacert" { continue; }
        let version = extract_version(store_path);
        yaml_pkgs.push(format!("  - {}", name));
        lock_pkgs.push(format!("  {}:\n      version: \"{}\"\n      store: \"{}\"", name, version, store_path));
    }

    yaml.push_str("# ws packages — managed by 'ws nix'\npackages:\n");
    yaml.push_str(&yaml_pkgs.join("\n"));
    yaml.push('\n');
    std::fs::write("ws.yaml", &yaml).map_err(|e| format!("write ws.yaml: {}", e))?;

    let mut lock = String::new();
    lock.push_str("# ws lockfile — auto-generated, commit this\n");
    if let Ok(rev) = get_nixpkgs_revision() {
        lock.push_str(&format!("nixpkgs_revision: \"{}\"\n", rev));
    }
    lock.push_str("packages:\n");
    lock.push_str(&lock_pkgs.join("\n"));
    lock.push('\n');
    std::fs::write("ws.lock", &lock).map_err(|e| format!("write ws.lock: {}", e))?;

    Ok(())
}

/// Get the pinned nixpkgs revision from an installed package's flake URL
fn get_nixpkgs_revision() -> Result<String, String> {
    // Use `nix flake metadata` to get the exact locked revision
    let output = nix_cmd()?
        .args(["flake", "metadata", "nixpkgs", "--json"])
        .output()
        .map_err(|e| format!("nix flake metadata: {}", e))?;
    let stdout = String::from_utf8_lossy(&output.stdout);
    let val: serde_json::Value = serde_json::from_str(&stdout)
        .map_err(|e| format!("parse flake metadata: {}", e))?;
    // The JSON has fields: url, locked, original, etc.
    if let Some(locked) = val.get("locked") {
        if let Some(rev) = locked.get("rev").and_then(|v| v.as_str()) {
            return Ok(rev.to_string());
        }
    }
    Err("no locked revision found".into())
}
