use std::collections::{HashMap, HashSet};
use std::path::Path;
use std::sync::{Arc, Mutex};
use std::time::{Duration, SystemTime};
use crate::lsp;

// ponytail: polling file watcher, no notify deps
//           2s poll, mtime cache, skips hidden/node_modules/target

struct PollWatcher {
    root: String,
    mtimes: HashMap<String, SystemTime>,
}

pub fn start_watch(root: &str) -> Result<(), String> {
    let root = Path::new(root).canonicalize().map_err(|e| format!("bad path: {}", e))?;
    if !root.is_dir() {
        return Err(format!("not a directory: {}", root.display()));
    }

    let root_str = root.to_string_lossy().to_string();

    // Phase 1: initial scan — detect languages, start LSP servers
    let (mtimes, file_samples) = initial_scan(&root);
    if !file_samples.is_empty() {
        eprintln!("ws: detected {} language(s), initializing LSP servers...", file_samples.len());
        let root_s = root.to_string_lossy().to_string();
        tokio::task::spawn_blocking(move || {
            let rt = tokio::runtime::Handle::current();
            for (ext, sample) in file_samples {
                eprintln!("ws:   initializing {} via {}", ext, sample);
                let result = rt.block_on(lsp::diagnose_file(&sample));
                match result {
                    Ok(diags) => {
                        for d in &diags {
                            let sev = ["", "ERROR", "WARN", "INFO", "HINT"].get(d.severity as usize).unwrap_or(&"?");
                            eprintln!("  {} {}:{}  {}", sev, d.line + 1, d.column + 1, d.message);
                        }
                    }
                    Err(e) => eprintln!("  lsp init error: {}", e),
                }
            }
            eprintln!("ws: LSP initialization complete for {}", root_s);
        });
    } else {
        eprintln!("ws: no recognized source files found in {}", root_str);
    }

    // Phase 2: file watching
    let watcher = Arc::new(Mutex::new(PollWatcher {
        root: root_str.clone(),
        mtimes,
    }));

    eprintln!("ws: watching {} for file changes", root_str);

    tokio::task::spawn_blocking(move || {
        let rt = tokio::runtime::Handle::current();
        let mut cleanup_counter = 0u32;

        loop {
            std::thread::sleep(Duration::from_millis(2000));

            cleanup_counter += 1;
            if cleanup_counter >= 15 {
                cleanup_counter = 0;
                let killed = lsp::cleanup_idle();
                if killed > 0 {
                    eprintln!("ws: cleaned up {} idle LSP session(s)", killed);
                }
            }

            let mut changed = Vec::new();
            let now = SystemTime::now();
            let (root, mut mtimes) = {
                let mut guard = watcher.lock().unwrap();
                (guard.root.clone(), std::mem::take(&mut guard.mtimes))
            };

            walk_dir(Path::new(&root), &mut mtimes, &now, &mut changed);

            watcher.lock().unwrap().mtimes = mtimes;

            for path in &changed {
                if !Path::new(path).exists() { continue; }
                eprintln!("ws: file changed: {}", path);
                rt.block_on(async {
                    match lsp::diagnose_file(path).await {
                        Ok(diags) => {
                            for d in &diags {
                                let sev = ["", "ERROR", "WARN", "INFO", "HINT"].get(d.severity as usize).unwrap_or(&"?");
                                eprintln!("  {} {}:{}  {}", sev, d.line + 1, d.column + 1, d.message);
                            }
                        }
                        Err(e) => eprintln!("  lsp error: {}", e),
                    }
                });
            }
        }
    });

    eprintln!("ws: watching {} ({} LSP servers loaded)", root_str, lsp::server::SERVERS.len());
    Ok(())
}

/// Walk project, collect mtimes + one sample file per extension
fn initial_scan(root: &Path) -> (HashMap<String, SystemTime>, Vec<(String, String)>) {
    let mut mtimes = HashMap::new();
    let mut seen_exts = HashSet::new();
    let mut samples = Vec::new();
    do_scan(root, &mut mtimes, &mut seen_exts, &mut samples);
    (mtimes, samples)
}

fn do_scan(dir: &Path, mtimes: &mut HashMap<String, SystemTime>,
           seen_exts: &mut HashSet<String>, samples: &mut Vec<(String, String)>) {
    let entries = match std::fs::read_dir(dir) {
        Ok(e) => e,
        Err(_) => return,
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            if let Some(name) = path.file_name().and_then(|n| n.to_str()) {
                if name.starts_with('.') || name == "node_modules" || name == "target" {
                    continue;
                }
            }
            do_scan(&path, mtimes, seen_exts, samples);
            continue;
        }
        if !path.is_file() { continue; }

        let path_str = path.to_string_lossy().to_string();
        let ext = path.extension()
            .and_then(|e| e.to_str())
            .map(|e| format!(".{}", e))
            .unwrap_or_default();
        let ext = if ext == "." && path.file_name().and_then(|n| n.to_str()) == Some("Dockerfile") {
            "Dockerfile".to_string()
        } else {
            ext
        };

        if ext.is_empty() || lsp::server::for_extension(&ext).is_empty() {
            continue;
        }

        if let Ok(meta) = path.metadata() {
            if let Ok(mtime) = meta.modified() {
                mtimes.insert(path_str, mtime);
            }
        }

        if seen_exts.insert(ext.clone()) {
            samples.push((ext, path.to_string_lossy().to_string()));
        }
    }
}

fn walk_dir(dir: &Path, mtimes: &mut HashMap<String, SystemTime>, now: &SystemTime, changed: &mut Vec<String>) {
    let entries = match std::fs::read_dir(dir) {
        Ok(e) => e,
        Err(_) => return,
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            if let Some(name) = path.file_name().and_then(|n| n.to_str()) {
                if name.starts_with('.') || name == "node_modules" || name == "target" {
                    continue;
                }
            }
            walk_dir(&path, mtimes, now, changed);
            continue;
        }
        if !path.is_file() { continue; }

        let path_str = path.to_string_lossy().to_string();
        let ext = path.extension()
            .and_then(|e| e.to_str())
            .map(|e| format!(".{}", e))
            .unwrap_or_default();
        let ext = if ext == "." && path.file_name().and_then(|n| n.to_str()) == Some("Dockerfile") {
            "Dockerfile".to_string()
        } else {
            ext
        };
        if ext.is_empty() || lsp::server::for_extension(&ext).is_empty() {
            continue;
        }

        let meta = match path.metadata() {
            Ok(m) => m,
            Err(_) => continue,
        };
        let mtime = match meta.modified() {
            Ok(t) => t,
            Err(_) => continue,
        };

        let prev = mtimes.get(&path_str).copied();
        if prev.map(|p| p != mtime).unwrap_or(true) {
            if prev.is_some() {
                changed.push(path_str.clone());
            }
            mtimes.insert(path_str, mtime);
        }
    }
}
