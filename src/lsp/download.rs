use std::path::PathBuf;

// ponytail: minimal download helpers; handles GitHub releases and npm
pub enum LspInstall {
    /// Download from GitHub releases (e.g. terraform-ls, rust-analyzer)
    GitHubRelease { repo: &'static str, name: &'static str },
    /// Install via npm (e.g. typescript-language-server, vue-language-server)
    Npm { package: &'static str, binary: &'static str },
    /// Install via cargo (e.g. rust-analyzer, taplo)
    Cargo { crate_name: &'static str, binary: &'static str },
    /// Expect the binary to already be on PATH
    System { binary: &'static str },
}

impl LspInstall {
    pub fn binary_name(&self) -> &str {
        match self {
            LspInstall::GitHubRelease { name, .. } => name,
            LspInstall::Npm { binary, .. } => binary,
            LspInstall::Cargo { binary, .. } => binary,
            LspInstall::System { binary } => binary,
        }
    }
}

/// The directory where downloaded LSP servers live
pub fn lsp_dir() -> PathBuf {
    let dir = std::env::temp_dir().join("ws-lsp");
    let _ = std::fs::create_dir_all(&dir);
    dir
}

/// Ensure an LSP binary is available, downloading/installing if needed.
/// Returns the path to the binary.
pub fn ensure(install: &LspInstall) -> Result<PathBuf, String> {
    let binary = install.binary_name();

    // Already on PATH?
    if let Ok(path) = which(binary) {
        return Ok(path);
    }

    // Already downloaded?
    let cached = lsp_dir().join(binary);
    if cached.exists() {
        return Ok(cached);
    }

    match install {
        LspInstall::System { .. } => {
            return Err(format!("{} not found on PATH. Please install it manually.", binary));
        }
        LspInstall::Npm { package, binary } => {
            install_npm(package, binary)?;
        }
        LspInstall::Cargo { crate_name, binary } => {
            install_cargo(crate_name, binary)?;
        }
        LspInstall::GitHubRelease { repo, name } => {
            download_github_release(repo, name)?;
        }
    }

    let cached = lsp_dir().join(binary);
    if cached.exists() {
        Ok(cached)
    } else {
        Err(format!("failed to find {} after installation", binary))
    }
}

fn which(name: &str) -> Result<PathBuf, String> {
    let output = std::process::Command::new("which")
        .arg(name)
        .output()
        .map_err(|e| format!("which failed: {}", e))?;
    if output.status.success() {
        let path = String::from_utf8_lossy(&output.stdout).trim().to_string();
        Ok(PathBuf::from(path))
    } else {
        Err(format!("{} not found", name))
    }
}

fn install_npm(package: &str, binary: &str) -> Result<(), String> {
    let target = lsp_dir();
    eprintln!("ws: installing {} via npm...", package);
    let status = std::process::Command::new("npm")
        .args(["install", "-g", "--prefix", &target.to_string_lossy(), package])
        .status()
        .map_err(|e| format!("npm install failed: {}", e))?;
    if !status.success() {
        return Err(format!("npm install {} failed", package));
    }
    // npm --prefix installs to <prefix>/bin/<binary>
    let bin_path = target.join("bin").join(binary);
    if bin_path.exists() {
        // symlink to lsp_dir for consistency
        let _ = std::fs::remove_file(target.join(binary));
        std::os::unix::fs::symlink(&bin_path, target.join(binary))
            .map_err(|e| format!("symlink failed: {}", e))?;
    }
    eprintln!("ws: {} installed", package);
    Ok(())
}

fn install_cargo(crate_name: &str, binary: &str) -> Result<(), String> {
    let target = lsp_dir();
    eprintln!("ws: installing {} via cargo...", crate_name);
    let status = std::process::Command::new("cargo")
        .args(["install", "--root", &target.to_string_lossy(), crate_name])
        .status()
        .map_err(|e| format!("cargo install failed: {}", e))?;
    if !status.success() {
        return Err(format!("cargo install {} failed", crate_name));
    }
    let bin_path = target.join("bin").join(binary);
    if bin_path.exists() {
        let _ = std::fs::remove_file(target.join(binary));
        std::os::unix::fs::symlink(&bin_path, target.join(binary))
            .map_err(|e| format!("symlink failed: {}", e))?;
    }
    eprintln!("ws: {} installed", crate_name);
    Ok(())
}

fn download_github_release(repo: &str, binary_name: &str) -> Result<(), String> {
    let target = lsp_dir();
    let os = std::env::consts::OS;
    let arch = std::env::consts::ARCH;

    // Map Rust arch/OS to GitHub release naming
    let gh_arch = match arch {
        "x86_64" => "amd64",
        "aarch64" => "arm64",
        _ => arch,
    };
    let gh_os = match os {
        "macos" => "darwin",
        "windows" => "windows",
        _ => os,
    };

    // Fetch latest release
    let url = format!("https://api.github.com/repos/{}/releases/latest", repo);
    let client = reqwest::blocking::Client::builder()
        .user_agent("ws-ide")
        .build()
        .map_err(|e| format!("http client: {}", e))?;
    let resp = client.get(&url).send().map_err(|e| format!("fetch releases: {}", e))?;
    let release: serde_json::Value = resp.json().map_err(|e| format!("parse release: {}", e))?;

    let tag = release["tag_name"].as_str().unwrap_or("latest");
    eprintln!("ws: downloading {} {} ({}/{})...", binary_name, tag, gh_os, gh_arch);

    // Find matching asset
    let assets = release["assets"].as_array().ok_or("no assets")?;
    let asset = assets.iter().find(|a| {
        let name = a["name"].as_str().unwrap_or("");
        let lower = name.to_lowercase();
        lower.contains(gh_arch) && lower.contains(gh_os) && lower.contains(binary_name)
    }).ok_or_else(|| format!("no matching asset for {}/{} in {}", gh_os, gh_arch, repo))?;

    let download_url = asset["browser_download_url"].as_str().ok_or("no download url")?;

    // Download
    let resp = client.get(download_url).send().map_err(|e| format!("download: {}", e))?;
    let bytes = resp.bytes().map_err(|e| format!("read: {}", e))?;

    let ext = if download_url.ends_with(".zip") { "zip" } else { "tar.gz" };
    let archive_path = target.join(format!("{}.{}", binary_name, ext));
    std::fs::write(&archive_path, &bytes).map_err(|e| format!("write: {}", e))?;

    // Extract
    if ext == "zip" {
        // unzip
        let status = std::process::Command::new("unzip")
            .args(["-o", &archive_path.to_string_lossy(), "-d", &target.to_string_lossy()])
            .status()
            .map_err(|e| format!("unzip: {}", e))?;
        if !status.success() {
            return Err("unzip failed".into());
        }
    } else {
        // tar xzf
        let status = std::process::Command::new("tar")
            .args(["xzf", &archive_path.to_string_lossy(), "-C", &target.to_string_lossy()])
            .status()
            .map_err(|e| format!("tar: {}", e))?;
        if !status.success() {
            return Err("tar extraction failed".into());
        }
    }

    // ponytail: find binary by name in extracted files, one level deep
    let _ = std::fs::remove_file(&archive_path);
    let found = find_extracted_binary(&target, binary_name);
    if let Some(src) = found {
        let _ = std::fs::remove_file(target.join(binary_name));
        let _ = std::fs::rename(&src, target.join(binary_name));
    }

    // Make executable
    let bin_path = target.join(binary_name);
    if bin_path.exists() {
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let perms = std::fs::metadata(&bin_path).map_err(|e| format!("metadata: {}", e))?.permissions();
            let mut perms = perms;
            perms.set_mode(0o755);
            std::fs::set_permissions(&bin_path, perms).map_err(|e| format!("chmod: {}", e))?;
        }
        eprintln!("ws: {} installed at {}", binary_name, bin_path.display());
        Ok(())
    } else {
        Err(format!("binary {} not found after extraction", binary_name))
    }
}

/// Find extracted binary by scanning target dir (flat + one level deep)
fn find_extracted_binary(dir: &std::path::Path, name: &str) -> Option<std::path::PathBuf> {
    let entries = std::fs::read_dir(dir).ok()?;
    for entry in entries.flatten() {
        let path = entry.path();
        let fname = path.file_name().and_then(|n| n.to_str()).unwrap_or("");
        if path.is_file() && fname.contains(name) && !fname.ends_with(".tar.gz") && !fname.ends_with(".zip") {
            return Some(path);
        }
    }
    // One level deep
    for entry in std::fs::read_dir(dir).ok()?.flatten() {
        let path = entry.path();
        if !path.is_dir() { continue; }
        for sub in std::fs::read_dir(&path).ok()?.flatten() {
            let sub_path = sub.path();
            let fname = sub_path.file_name().and_then(|n| n.to_str()).unwrap_or("");
            if sub_path.is_file() && fname.contains(name) && !fname.ends_with(".tar.gz") && !fname.ends_with(".zip") {
                return Some(sub_path);
            }
        }
    }
    None
}
