/// Nix package manager — host daemon, shared store, per-HOME profiles
/// ponytail: each container has its own HOME → own profile. No custom dirs.
use std::path::Path;
use std::process::Command;

const NIX_BIN: &str = "/nix/var/nix/profiles/default/bin/nix";
const NIX_PROFILE_BIN: &str = "/nix/var/nix/profiles/default/bin";

/// Add nix binaries to PATH at startup
pub fn setup_env() {
    let mut extra = Vec::new();
    // Default nix profile bin (for the nix CLI itself)
    extra.push(NIX_PROFILE_BIN.to_string());
    // Per-user profile (for installed packages)
    let user_profile = "/nix/var/nix/profiles/per-user/root/profile/bin";
    if Path::new(user_profile).exists() {
        extra.push(user_profile.to_string());
    }
    // Home-manager profile
    let home_profile = "/root/.nix-profile/bin";
    if Path::new(home_profile).exists() {
        extra.push(home_profile.to_string());
    }
    if let Ok(current) = std::env::var("PATH") {
        let new_path = extra.join(":") + ":" + &current;
        std::env::set_var("PATH", &new_path);
    }
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

fn nix_cmd() -> Result<Command, String> {
    let nix = find_nix()
        .ok_or_else(|| "Nix not found. Mount /nix from host: -v /nix:/nix".to_string())?;
    let mut cmd = Command::new(&nix);
    cmd.env("NIX_REMOTE", "daemon");
    cmd.arg("--extra-experimental-features");
    cmd.arg("nix-command flakes");
    Ok(cmd)
}

/// Install a package (goes to this container's default user profile)
pub fn install(package: &str) -> Result<String, String> {
    let attr = format!("nixpkgs#{}", package);
    eprintln!("ws: nix installing {}...", package);
    let output = nix_cmd()?
        .args(["profile", "install", &attr])
        .output()
        .map_err(|e| format!("nix install: {}", e))?;
    if output.status.success() {
        eprintln!("ws: package {} installed", package);
        Ok("done".into())
    } else {
        Err(String::from_utf8_lossy(&output.stderr).trim().to_string())
    }
}

pub fn search(query: &str) -> Result<Vec<String>, String> {
    let output = nix_cmd()?
        .args(["search", "nixpkgs", query])
        .output()
        .map_err(|e| format!("nix search: {}", e))?;
    Ok(String::from_utf8_lossy(&output.stdout)
        .lines()
        .map(|l| l.to_string())
        .collect())
}

pub fn list() -> Result<Vec<String>, String> {
    let output = nix_cmd()?
        .args(["profile", "list", "--json"])
        .output()
        .map_err(|e| format!("nix list: {}", e))?;
    let stdout = String::from_utf8_lossy(&output.stdout);
    // JSON format: {"elements":{"name":{...}},"version":3}
    match serde_json::from_str::<serde_json::Value>(&stdout) {
        Ok(val) => {
            let names: Vec<String> = val
                .get("elements")
                .and_then(|e| e.as_object())
                .map(|obj| obj.keys().cloned().collect())
                .unwrap_or_default();
            Ok(names)
        }
        Err(_) => Ok(vec![]),
    }
}

pub fn remove(package: &str) -> Result<String, String> {
    let output = nix_cmd()?
        .args(["profile", "remove", package])
        .output()
        .map_err(|e| format!("nix remove: {}", e))?;
    if output.status.success() {
        Ok("removed".into())
    } else {
        Err(String::from_utf8_lossy(&output.stderr).trim().to_string())
    }
}
