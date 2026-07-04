//! Nix package manager — host daemon, shared store, per-HOME profiles
//! Supports ws.yaml + ws.lock for reproducible workspaces, package@version syntax

use std::collections::{HashMap, HashSet};
use std::path::Path;
use std::process::Command;
use std::sync::Mutex;

const NIX_BIN: &str = "/nix/var/nix/profiles/default/bin/nix";
const NIX_PROFILE_BIN: &str = "/nix/var/nix/profiles/default/bin";

/// Packages explicitly installed by the user (pkg_install), not auto-installed by LSP.
fn explicit_packages() -> &'static Mutex<HashSet<String>> {
    static PKGS: std::sync::OnceLock<Mutex<HashSet<String>>> = std::sync::OnceLock::new();
    PKGS.get_or_init(|| Mutex::new(HashSet::new()))
}

/// Ensure Nix is available and user profile symlink is valid.
/// Called at server startup.
pub fn setup_env() {
    if let Ok(current) = std::env::var("PATH") {
        if !current.contains(NIX_PROFILE_BIN) {
            let new_path = format!("{}:{}", NIX_PROFILE_BIN, current);
            std::env::set_var("PATH", &new_path);
        }
    }
    // Fix user nix profile symlink — point to profiles/profile so nix's own
    // updates (from `nix profile add`) propagate automatically.
    // The with-s link (~/.local/state/nix/profiles/profile) is updated by nix;
    // the without-s link (~/.local/state/nix/profile) is stale at container build.
    if let Ok(home) = std::env::var("HOME") {
        let profile_link = format!("{}/.local/state/nix/profile", home);
        let profiles_profile = format!("{}/.local/state/nix/profiles/profile", home);
        match std::fs::read_link(&profile_link) {
            Ok(current) if current.to_string_lossy() == profiles_profile => {}
            _ => {
                let _ = std::fs::remove_file(&profile_link);
                let _ = std::os::unix::fs::symlink(&profiles_profile, &profile_link);
            }
        }
        // Also add user nix profile bin to PATH so installed tools are found
        let user_nix_bin = format!("{}/.local/state/nix/profile/bin", home);
        let current = std::env::var("PATH").unwrap_or_default();
        if !current.contains(&user_nix_bin) {
            std::env::set_var("PATH", format!("{}:{}", user_nix_bin, current));
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

/// Install a package (user-requested). Marks it for ws.yaml.
pub fn install(input: &str) -> Result<String, String> {
    let (name, _) = input.split_once('@').unwrap_or((input, ""));
    explicit_packages().lock().unwrap().insert(name.to_string());
    install_inner(input)
}

/// Install a package (auto-dependency, e.g. LSP). Skips ws.yaml.
pub fn install_auto(input: &str) -> Result<String, String> {
    install_inner(input)
}

fn install_inner(input: &str) -> Result<String, String> {
    let (name, version) = input.split_once('@').unwrap_or((input, ""));
    if version.is_empty() {
        install_one(&resolve_flake(), name)
    } else {
        install_version(name, version)
    }
}

fn install_one(flake: &str, pkg: &str) -> Result<String, String> {
    let attr = format!("{}#{}", flake, pkg);
    eprintln!("ws: nix installing {}...", attr);
    let output = nix_cmd()?
        .args(["profile", "add", &attr])
        .output()
        .map_err(|e| format!("nix install: {}", e))?;
    if output.status.success() {
        eprintln!("ws: package {} installed", pkg);
        let _ = write_workspace_files();
        Ok(format!(
            "installed {}: {}",
            pkg,
            String::from_utf8_lossy(&output.stderr)
                .lines()
                .last()
                .unwrap_or("")
        ))
    } else {
        Err(String::from_utf8_lossy(&output.stderr).trim().to_string())
    }
}

fn install_version(name: &str, version: &str) -> Result<String, String> {
    let flake = resolve_flake();
    // Try candidates: pkg@version, pkg_version, pkg_version_no_dots
    let candidates = vec![
        format!("{}#{}@{}", flake, name, version),
        format!("{}#{}_{}", flake, name, version.replace('.', "_")),
        format!("{}#{}_{}", flake, name, version.replace('.', "")),
    ];
    for attr in &candidates {
        eprintln!("ws: trying {}...", attr);
        let output = nix_cmd()?
            .args(["profile", "add", attr])
            .output()
            .map_err(|e| format!("nix install: {}", e))?;
        if output.status.success() {
            eprintln!("ws: package {}@{} installed", name, version);
            let _ = write_workspace_files();
            return Ok(format!(
                "installed {}@{}: {}",
                name,
                version,
                String::from_utf8_lossy(&output.stderr)
                    .lines()
                    .last()
                    .unwrap_or("")
            ));
        }
    }
    Err(format!("no version {}@{} found in nixpkgs", name, version))
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

pub fn sync() -> Result<String, String> {
    write_workspace_files()?;
    Ok("ws.yaml + ws.lock updated".into())
}

pub fn search(query: &str) -> Result<Vec<String>, String> {
    let output = nix_cmd()?
        .args(["search", "nixpkgs", query])
        .output()
        .map_err(|e| format!("nix search: {}", e))?;
    // ponytail: strip ANSI escape codes from nix search output
    let raw = String::from_utf8_lossy(&output.stdout);
    let mut cleaned = String::with_capacity(raw.len());
    let mut in_escape = false;
    for ch in raw.chars() {
        if in_escape {
            if ch == 'm' {
                in_escape = false;
            }
        } else if ch == '\x1b' {
            in_escape = true;
        } else {
            cleaned.push(ch);
        }
    }
    Ok(cleaned.lines().map(|l| l.to_string()).collect())
}

pub fn list() -> Result<Vec<String>, String> {
    get_installed().map(|m| m.into_keys().collect())
}

fn get_installed() -> Result<HashMap<String, String>, String> {
    let val = get_profile_json()?;
    let mut map = HashMap::new();
    if let Some(elements) = val.get("elements").and_then(|e| e.as_object()) {
        for (name, info) in elements {
            let store = info
                .get("storePaths")
                .and_then(|p| p.as_array())
                .and_then(|a| a.first())
                .and_then(|v| v.as_str())
                .unwrap_or("");
            map.insert(name.clone(), store.to_string());
        }
    }
    Ok(map)
}

fn get_profile_json() -> Result<serde_json::Value, String> {
    let output = nix_cmd()?
        .args(["profile", "list", "--json"])
        .output()
        .map_err(|e| format!("nix list: {}", e))?;
    serde_json::from_str(&String::from_utf8_lossy(&output.stdout))
        .map_err(|e| format!("parse: {}", e))
}

fn extract_version(store_path: &str) -> String {
    let name = store_path.rsplit('/').next().unwrap_or("");
    let parts: Vec<&str> = name.splitn(2, '-').collect();
    if parts.len() < 2 {
        return "".into();
    }
    let rest = parts[1];
    let dash = rest.find('-').unwrap_or(rest.len());
    rest[dash + 1..].to_string()
}

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

pub fn apply_yaml() -> Result<String, String> {
    let content =
        std::fs::read_to_string("ws.yaml").map_err(|e| format!("cannot read ws.yaml: {}", e))?;
    let mut desired = Vec::new();
    for line in content.lines() {
        let t = line.trim();
        if let Some(name) = t.strip_prefix("- ") {
            if !name.is_empty() {
                desired.push(name.to_string());
            }
        }
    }
    let current = list().unwrap_or_default();
    let mut installed = Vec::new();
    for pkg in &desired {
        // ponytail: pkg may be "go@1.23.4" — list() returns bare names
        let name = pkg.split_once('@').map(|(n, _)| n).unwrap_or(pkg);
        if current.iter().any(|c| c == name) {
            eprintln!("ws: {} already installed", pkg);
        } else {
            match install(name) {
                Ok(m) => installed.push(format!("  {}: {}", pkg, m)),
                Err(e) => eprintln!("ws: {} install failed: {}", pkg, e),
            }
        }
    }
    if installed.is_empty() {
        Ok("all packages already installed".into())
    } else {
        Ok(format!("installed:\n{}", installed.join("\n")))
    }
}

fn write_workspace_files() -> Result<(), String> {
    let pkgs = get_installed()?;
    let explicit = explicit_packages().lock().unwrap();
    let mut yaml = String::new();
    let mut lock_pkgs = Vec::new();
    let mut yaml_pkgs = Vec::new();
    for (name, store_path) in &pkgs {
        if name == "nix" || name == "nix-manual" || name == "nss-cacert" {
            continue;
        }
        let version = extract_version(store_path);
        // ws.lock tracks everything for reproducibility
        lock_pkgs.push(format!(
            "  {}:\n      version: \"{}\"\n      store: \"{}\"",
            name, version, store_path
        ));
        // ws.yaml: only explicit user-installed packages
        if !explicit.contains(name.as_str()) {
            continue;
        }
        let yaml_entry = format!("  - {}", name);
        yaml_pkgs.push(yaml_entry);
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

fn get_nixpkgs_revision() -> Result<String, String> {
    let output = nix_cmd()?
        .args(["flake", "metadata", "nixpkgs", "--json"])
        .output()
        .map_err(|e| format!("nix flake metadata: {}", e))?;
    let val: serde_json::Value = serde_json::from_str(&String::from_utf8_lossy(&output.stdout))
        .map_err(|e| format!("parse: {}", e))?;
    if let Some(locked) = val.get("locked") {
        if let Some(rev) = locked.get("rev").and_then(|v| v.as_str()) {
            return Ok(rev.to_string());
        }
    }
    Err("revision not found".into())
}
