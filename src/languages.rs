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
        Some("sql") => "sql",
        Some("md") => "markdown",
        Some("toml") => "toml",
        Some("sh" | "bash" | "zsh") => "shellscript",
        Some("html") => "html",
        Some("css") => "css",
        _ => "plaintext",
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn sql_files_share_their_language_identifier_with_native_extension_selectors() {
        assert_eq!(language(Path::new("queries/query.sql")), "sql");
        assert_eq!(language(Path::new("queries/query.sql.txt")), "plaintext");
    }
}
