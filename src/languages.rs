use std::path::Path;

// Shared file-extension language identifiers for native services.
pub fn language(path: &Path) -> &'static str {
    match path.extension().and_then(|s| s.to_str()) {
        Some("rs") => "rust",
        Some("py") => "python",
        Some("c" | "h") => "c",
        Some("cpp" | "cc" | "cxx" | "hpp") => "cpp",
        Some("js" | "jsx") => "javascript",
        Some("ts" | "tsx") => "typescript",
        Some("go") => "go",
        Some("json") => "json",
        Some("md") => "markdown",
        Some("toml") => "toml",
        Some("sh" | "bash" | "zsh") => "shellscript",
        Some("html") => "html",
        Some("css") => "css",
        _ => "plaintext",
    }
}
