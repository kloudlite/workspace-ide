// ponytail: binary name only — install via nix if not on PATH

#[derive(Clone)]
pub struct LspServer {
    pub id: &'static str,
    pub language_id: &'static str,
    pub extensions: &'static [&'static str],
    pub binary: &'static str,
    pub args: &'static [&'static str],
    pub needs_lockfile: bool,
    pub nix_packages: &'static [&'static str],
}

// ponytail: trimmed to servers actually used on kmac (Go mono-repo).
// Add more as needed: zls, elixir-ls, php, svelte, astro, vue,
// dockerfile-ls, terraform, css-ls, html-ls available in nixpkgs.
pub static SERVERS: &[LspServer] = &[
    LspServer {
        id: "typescript",
        language_id: "typescript",
        extensions: &[".ts", ".tsx", ".js", ".jsx", ".mjs", ".cjs", ".mts", ".cts"],
        binary: "typescript-language-server",
        args: &["--stdio"],
        needs_lockfile: true,
        nix_packages: &["nodejs"],
    },
    LspServer {
        id: "rust",
        language_id: "rust",
        extensions: &[".rs"],
        binary: "rust-analyzer",
        args: &[],
        needs_lockfile: true,
        nix_packages: &["cargo", "rustc"],
    },
    LspServer {
        id: "gopls",
        language_id: "go",
        extensions: &[".go"],
        binary: "gopls",
        args: &[],
        needs_lockfile: true,
        nix_packages: &["go"],
    },
    LspServer {
        id: "pyright",
        language_id: "python",
        extensions: &[".py", ".pyi"],
        binary: "pyright-langserver",
        args: &["--stdio"],
        needs_lockfile: true,
        nix_packages: &["python3"],
    },
    LspServer {
        id: "clangd",
        language_id: "cpp",
        extensions: &[".c", ".cpp", ".cc", ".cxx", ".h", ".hpp", ".hh", ".hxx"],
        binary: "clangd",
        args: &[],
        needs_lockfile: true,
        nix_packages: &["clang-tools"],
    },
    LspServer {
        id: "lua-ls",
        language_id: "lua",
        extensions: &[".lua"],
        binary: "lua-language-server",
        args: &[],
        needs_lockfile: false,
        nix_packages: &[],
    },
    LspServer {
        id: "bashls",
        language_id: "shellscript",
        extensions: &[".sh", ".bash", ".zsh", ".ksh"],
        binary: "bash-language-server",
        args: &["start"],
        needs_lockfile: false,
        nix_packages: &[],
    },
    LspServer {
        id: "yaml-ls",
        language_id: "yaml",
        extensions: &[".yaml", ".yml"],
        binary: "yaml-language-server",
        args: &["--stdio"],
        needs_lockfile: false,
        nix_packages: &[],
    },
    LspServer {
        id: "json-ls",
        language_id: "json",
        extensions: &[".json", ".jsonc"],
        binary: "json-languageserver",
        args: &["--stdio"],
        needs_lockfile: false,
        nix_packages: &[],
    },
];

pub fn for_extension(ext: &str) -> Vec<&'static LspServer> {
    SERVERS
        .iter()
        .filter(|s| s.extensions.contains(&ext))
        .collect()
}
