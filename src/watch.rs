use crate::lsp;
use crate::lsp::walk_files;
use std::collections::{HashMap, HashSet};
use std::path::Path;
use std::sync::{Arc, Mutex};
use std::time::{Duration, SystemTime};

// ponytail: polling file watcher, no notify deps
//           2s poll, mtime cache, skips hidden/node_modules/target

struct PollWatcher {
    root: String,
    mtimes: HashMap<String, SystemTime>,
}

pub fn start_watch(root: &str) {
    let root = match Path::new(root).canonicalize() {
        Ok(r) => r,
        Err(e) => {
            eprintln!("ws: watch error: {}", e);
            return;
        }
    };
    if !root.is_dir() {
        eprintln!("ws: not a directory: {}", root.display());
        return;
    }

    let root_str = root.to_string_lossy().to_string();

    // Phase 1: initial scan — detect languages, start LSP servers
    let (mtimes, file_samples) = initial_scan(&root);
    if !file_samples.is_empty() {
        eprintln!(
            "ws: detected {} language(s), initializing LSP servers...",
            file_samples.len()
        );
        let root_s = root.to_string_lossy().to_string();
        tokio::task::spawn_blocking(move || {
            let rt = tokio::runtime::Handle::current();
            for (ext, sample) in file_samples {
                eprintln!("ws:   initializing {} via {}", ext, sample);
                let result = rt.block_on(lsp::diagnose_file(&sample));
                match result {
                    Ok(diags) => {
                        for d in &diags {
                            eprintln!(
                                "  {} {}:{}  {}",
                                severity_label(d.severity),
                                d.line + 1,
                                d.column + 1,
                                d.message
                            );
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
        let mut reconcile_counter = 0u64;

        loop {
            std::thread::sleep(Duration::from_millis(2000));

            cleanup_counter += 1;
            reconcile_counter += 1;
            if cleanup_counter >= 15 {
                cleanup_counter = 0;
                let killed = lsp::cleanup_idle();
                if killed > 0 {
                    eprintln!("ws: cleaned up {} idle LSP session(s)", killed);
                }
            }
            // ponytail: reconcile every ~10min (300 ticks × 2s)
            if reconcile_counter % 300 == 0 {
                let (added, removed) = lsp::reconcile_lsp();
                if added > 0 || removed > 0 {
                    eprintln!(
                        "ws: LSP reconcile — installed {}, uninstalled {}",
                        added, removed
                    );
                }
            }

            let mut changed = Vec::new();
            let (root, mut mtimes) = {
                let mut guard = watcher.lock().unwrap();
                (guard.root.clone(), std::mem::take(&mut guard.mtimes))
            };

            walk_dir(Path::new(&root), &mut mtimes, &mut changed);

            watcher.lock().unwrap().mtimes = mtimes;

            for path in &changed {
                if !Path::new(path).exists() {
                    continue;
                }
                eprintln!("ws: file changed: {}", path);
                rt.block_on(async {
                    match lsp::diagnose_file(path).await {
                        Ok(diags) => {
                            for d in &diags {
                                eprintln!(
                                    "  {} {}:{}  {}",
                                    severity_label(d.severity),
                                    d.line + 1,
                                    d.column + 1,
                                    d.message
                                );
                            }
                        }
                        Err(e) => eprintln!("  lsp error: {}", e),
                    }
                });
            }
        }
    });

    eprintln!(
        "ws: watching {} ({} LSP servers loaded)",
        root_str,
        lsp::server::SERVERS.len()
    );
}

fn severity_label(s: u8) -> &'static str {
    ["", "ERROR", "WARN", "INFO", "HINT"]
        .get(s as usize)
        .unwrap_or(&"?")
}

fn initial_scan(root: &Path) -> (HashMap<String, SystemTime>, Vec<(String, String)>) {
    let mut mtimes = HashMap::new();
    let mut seen_exts = HashSet::new();
    let mut samples = Vec::new();
    walk_files(root, &mut |path| {
        let path_str = path.to_string_lossy().to_string();
        let ext = lsp::extension_for(&path_str);
        if let Ok(meta) = path.metadata() {
            if let Ok(mtime) = meta.modified() {
                mtimes.insert(path_str.clone(), mtime);
            }
        }
        if seen_exts.insert(ext.to_string()) {
            samples.push((ext.to_string(), path_str));
        }
    });
    (mtimes, samples)
}

fn walk_dir(dir: &Path, mtimes: &mut HashMap<String, SystemTime>, changed: &mut Vec<String>) {
    walk_files(dir, &mut |path| {
        let path_str = path.to_string_lossy().to_string();
        let meta = match path.metadata() {
            Ok(m) => m,
            Err(_) => return,
        };
        let mtime = match meta.modified() {
            Ok(t) => t,
            Err(_) => return,
        };
        let prev = mtimes.get(&path_str).copied();
        if prev.map(|p| p != mtime).unwrap_or(true) {
            if prev.is_some() {
                changed.push(path_str.clone());
            }
            mtimes.insert(path_str, mtime);
        }
    });
}
