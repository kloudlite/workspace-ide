// ponytail: binary name only — install via nix if not on PATH

pub struct LspServer {
    pub id: &'static str,
    pub extensions: &'static [&'static str],
    pub binary: &'static str,
    pub args: &'static [&'static str],
    pub needs_lockfile: bool,
}

pub static SERVERS: &[LspServer] = &[
    LspServer {
        id: "typescript",
        extensions: &[".ts", ".tsx", ".js", ".jsx", ".mjs", ".cjs", ".mts", ".cts"],
        binary: "typescript-language-server",
        args: &["--stdio"],
        needs_lockfile: true,
    },
    LspServer {
        id: "rust",
        extensions: &[".rs"],
        binary: "rust-analyzer",
        args: &[],
        needs_lockfile: false,
    },
    LspServer {
        id: "gopls",
        extensions: &[".go"],
        binary: "gopls",
        args: &[],
        needs_lockfile: false,
    },
    LspServer {
        id: "pyright",
        extensions: &[".py", ".pyi"],
        binary: "pyright-langserver",
        args: &["--stdio"],
        needs_lockfile: false,
    },
    LspServer {
        id: "clangd",
        extensions: &[".c", ".cpp", ".cc", ".cxx", ".h", ".hpp", ".hh", ".hxx"],
        binary: "clangd",
        args: &[],
        needs_lockfile: false,
    },
    LspServer {
        id: "lua-ls",
        extensions: &[".lua"],
        binary: "lua-language-server",
        args: &[],
        needs_lockfile: false,
    },
    LspServer {
        id: "bashls",
        extensions: &[".sh", ".bash", ".zsh", ".ksh"],
        binary: "bash-language-server",
        args: &["start"],
        needs_lockfile: false,
    },
    LspServer {
        id: "yaml-ls",
        extensions: &[".yaml", ".yml"],
        binary: "yaml-language-server",
        args: &["--stdio"],
        needs_lockfile: false,
    },
    LspServer {
        id: "json-ls",
        extensions: &[".json", ".jsonc"],
        binary: "json-languageserver",
        args: &["--stdio"],
        needs_lockfile: false,
    },
    LspServer {
        id: "dockerfile-ls",
        extensions: &["Dockerfile", ".dockerfile"],
        binary: "dockerfile-langserver",
        args: &["--stdio"],
        needs_lockfile: false,
    },
    LspServer {
        id: "terraform",
        extensions: &[".tf", ".tfvars"],
        binary: "terraform-ls",
        args: &["serve"],
        needs_lockfile: false,
    },
    LspServer {
        id: "svelte",
        extensions: &[".svelte"],
        binary: "svelte-language-server",
        args: &["--stdio"],
        needs_lockfile: true,
    },
    LspServer {
        id: "vue",
        extensions: &[".vue"],
        binary: "vue-language-server",
        args: &["--stdio"],
        needs_lockfile: true,
    },
    LspServer {
        id: "astro",
        extensions: &[".astro"],
        binary: "astro-ls",
        args: &["--stdio"],
        needs_lockfile: true,
    },
    LspServer {
        id: "css-ls",
        extensions: &[".css", ".scss", ".less"],
        binary: "css-languageserver",
        args: &["--stdio"],
        needs_lockfile: false,
    },
    LspServer {
        id: "html-ls",
        extensions: &[".html", ".htm"],
        binary: "html-languageserver",
        args: &["--stdio"],
        needs_lockfile: false,
    },
    LspServer {
        id: "zls",
        extensions: &[".zig", ".zon"],
        binary: "zls",
        args: &[],
        needs_lockfile: false,
    },
    LspServer {
        id: "elixir-ls",
        extensions: &[".ex", ".exs"],
        binary: "elixir-ls",
        args: &[],
        needs_lockfile: false,
    },
    LspServer {
        id: "php",
        extensions: &[".php"],
        binary: "intelephense",
        args: &["--stdio"],
        needs_lockfile: false,
    },
];

pub fn for_extension(ext: &str) -> Vec<&'static LspServer> {
    SERVERS
        .iter()
        .filter(|s| s.extensions.contains(&ext))
        .collect()
}

pub fn language_for_ext(ext: &str) -> &'static str {
    match ext {
        ".rs" => "rust",
        ".go" => "go",
        ".py" | ".pyi" => "python",
        ".ts" | ".tsx" => "typescript",
        ".js" | ".jsx" => "javascript",
        ".mjs" | ".cjs" => "javascript",
        ".mts" | ".cts" => "typescript",
        ".json" | ".jsonc" => "json",
        ".yaml" | ".yml" => "yaml",
        ".css" => "css",
        ".scss" => "scss",
        ".less" => "less",
        ".html" | ".htm" => "html",
        ".sh" | ".bash" | ".zsh" => "shellscript",
        ".c" => "c",
        ".cpp" | ".cc" | ".cxx" | ".hpp" => "cpp",
        ".h" | ".hh" | ".hxx" => "c",
        ".lua" => "lua",
        ".php" => "php",
        ".rb" => "ruby",
        ".java" => "java",
        ".kt" | ".kts" => "kotlin",
        ".swift" => "swift",
        ".zig" | ".zon" => "zig",
        ".svelte" => "svelte",
        ".vue" => "vue",
        ".astro" => "astro",
        ".tf" | ".tfvars" => "terraform",
        ".ex" | ".exs" => "elixir",
        _ => "",
    }
}
