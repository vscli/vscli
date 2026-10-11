//! Exact bounded creation inputs, independent of editor focus or mirror generation.
use super::*;
use anyhow::ensure;
#[derive(Clone)]
pub(crate) struct CreationContext {
    root: PathBuf,
    settings: Arc<Vec<serde_json::Map<String, Value>>>,
    profile: u64,
}
impl CreationContext {
    pub(crate) fn new(
        root: PathBuf,
        settings: Arc<Vec<serde_json::Map<String, Value>>>,
        profile: u64,
    ) -> Result<Self> {
        ensure!(
            root.is_absolute() && root.as_os_str().len() <= 4096,
            "Invalid untitled workspace context"
        );
        Ok(Self {
            root,
            settings,
            profile,
        })
    }
    fn current(
        &self,
        root: &Path,
        settings: &Arc<Vec<serde_json::Map<String, Value>>>,
        profile: u64,
    ) -> bool {
        self.root == root && Arc::ptr_eq(&self.settings, settings) && self.profile == profile
    }
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Arguments {
    uri: Option<String>,
    content: String,
    language: Option<String>,
}
pub(crate) struct Capture {
    pub(crate) uri: Option<String>,
    pub(crate) requested_uri: bool,
    pub(crate) content: String,
    pub(crate) language: Option<String>,
    pub(crate) associated_path: Option<PathBuf>,
    context: CreationContext,
}
impl Capture {
    pub(crate) fn new(context: CreationContext, args: &Value) -> Result<Arc<Self>> {
        let object = args
            .as_object()
            .context("Invalid untitled creation arguments")?;
        ensure!(
            object.len() == 3
                && ["uri", "content", "language"]
                    .iter()
                    .all(|key| object.contains_key(*key)),
            "Untitled creation requires exact fields"
        );
        let arguments: Arguments = serde_json::from_value(args.clone())?;
        ensure!(
            arguments.content.len() <= 1024 * 1024 && !arguments.content.contains('\0'),
            "Untitled content exceeds 1 MiB or contains NUL"
        );
        ensure!(
            arguments
                .language
                .as_ref()
                .is_none_or(|language| !language.is_empty()
                    && language.len() <= 128
                    && !language.contains('\0')),
            "Invalid untitled language identifier"
        );
        let associated_path = if let Some(uri) = &arguments.uri {
            ensure!(
                uri.len() <= 4096 && !uri.contains('\0'),
                "Untitled URI exceeds 4096 bytes"
            );
            let parsed = url::Url::parse(uri)?;
            ensure!(
                parsed.scheme() == "untitled"
                    && parsed.query().is_none()
                    && parsed.fragment().is_none(),
                "Unsupported untitled URI"
            );
            if parsed.path().starts_with('/') {
                let file_uri = format!(
                    "file:{}",
                    uri.strip_prefix("untitled:")
                        .context("Invalid untitled scheme")?
                );
                let path = url::Url::parse(&file_uri)?
                    .to_file_path()
                    .map_err(|_| anyhow::anyhow!("Unsupported associated file authority"))?;
                ensure!(
                    path.is_absolute()
                        && path.as_os_str().len() <= 4096
                        && path.file_name().is_some(),
                    "Invalid associated untitled destination"
                );
                Some(path)
            } else {
                ensure!(
                    !parsed.path().is_empty() && parsed.host_str().is_none(),
                    "Untitled URI requires a name or associated path"
                );
                None
            }
        } else {
            None
        };
        ensure!(
            arguments.uri.is_none()
                || (arguments.content.is_empty() && arguments.language.is_none()),
            "URI opening cannot supply initial content or language"
        );
        let language = if let Some(language) = arguments.language {
            Some(language)
        } else if let Some(path) = &associated_path {
            Some(lsp::language(path).into())
        } else {
            context
                .settings
                .iter()
                .rev()
                .find_map(|layer| layer.get("files.defaultLanguage"))
                .and_then(Value::as_str)
                .filter(|language| {
                    !language.is_empty() && language.len() <= 128 && !language.contains('\0')
                })
                .map(str::to_owned)
        };
        Ok(Arc::new(Self {
            requested_uri: arguments.uri.is_some(),
            uri: arguments.uri,
            content: arguments.content,
            language,
            associated_path,
            context,
        }))
    }
    pub(crate) fn current(
        &self,
        root: &Path,
        settings: &Arc<Vec<serde_json::Map<String, Value>>>,
        profile: u64,
    ) -> bool {
        self.context.current(root, settings, profile)
    }
    pub(crate) fn prepare(&self) -> Result<Document> {
        if let Some(path) = &self.associated_path {
            match std::fs::symlink_metadata(path) {
                Ok(_) => bail!("Associated untitled file already exists"),
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
                Err(error) => {
                    return Err(error).context("Cannot inspect associated untitled destination");
                }
            }
        }
        Document::from_untitled(
            self.uri.clone(),
            &self.content,
            self.language.clone(),
            self.associated_path.clone(),
        )
    }
}
impl Client {
    pub(crate) fn capture_untitled(
        &mut self,
        args: &Value,
        documents: &[Document],
        hidden: &[Document],
    ) -> Result<Arc<Capture>> {
        let context = self
            .untitled_context
            .clone()
            .context("Untitled creation context unavailable")?;
        let mut input = Capture::new(context, args)?;
        if input.uri.is_none() {
            ensure!(
                documents.len().saturating_add(hidden.len()) <= 128,
                "Untitled retained resource inventory exceeds 128"
            );
            loop {
                self.next_untitled_name = self
                    .next_untitled_name
                    .checked_add(1)
                    .filter(|value| *value < (1 << 53))
                    .context("Untitled resource identity exhausted")?;
                let uri = format!("untitled:Untitled-{}", self.next_untitled_name);
                let used = documents.iter().chain(hidden).any(|doc| doc.untitled_uri() == Some(uri.as_str()))
                    || self.services.iter().any(|service| matches!(&service.operation, services::Operation::UntitledOpen(input) if input.uri.as_deref() == Some(uri.as_str())));
                if !used {
                    Arc::get_mut(&mut input)
                        .context("Untitled fresh capture was shared")?
                        .uri = Some(uri);
                    break;
                }
            }
        }
        Ok(input)
    }
    pub(crate) fn set_untitled_context(&mut self, context: Option<CreationContext>) {
        self.untitled_context = context;
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    fn context(root: &Path) -> CreationContext {
        CreationContext::new(root.into(), Arc::new(vec![]), 1).unwrap()
    }
    fn arguments() -> Value {
        json!({"uri":null,"content":"猫🙂\r\n","language":"cpp"})
    }
    #[test]
    fn exact_inputs_and_original_creation_context_are_retained() {
        let root = tempfile::tempdir().unwrap();
        let ctx = context(root.path());
        let capture = Capture::new(ctx.clone(), &arguments()).unwrap();
        assert!(capture.current(root.path(), &ctx.settings, 1));
        assert!(!capture.current(root.path(), &Arc::new(vec![]), 1));
        assert!(!capture.current(root.path(), &ctx.settings, 2));
        let mut invalid = arguments();
        invalid["extra"] = true.into();
        assert!(Capture::new(ctx.clone(), &invalid).is_err());
        invalid = arguments();
        invalid["language"] = "".into();
        assert!(Capture::new(ctx.clone(), &invalid).is_err());
        invalid = arguments();
        invalid["content"] = "猫".repeat(400_000).into();
        assert!(Capture::new(ctx.clone(), &invalid).is_err());
        invalid = arguments();
        invalid.as_object_mut().unwrap().remove("uri");
        assert!(Capture::new(ctx, &invalid).is_err());
    }
    #[test]
    fn associated_creation_never_loads_or_overwrites_existing_files_or_symlinks() {
        let root = tempfile::tempdir().unwrap();
        let path = root.path().join("new.cpp");
        let uri = lsp::file_uri(&path)
            .unwrap()
            .replacen("file:", "untitled:", 1);
        let capture = Capture::new(
            context(root.path()),
            &json!({"uri":uri,"content":"","language":null}),
        )
        .unwrap();
        let doc = capture.prepare().unwrap();
        assert!(doc.path.is_none());
        assert_eq!(doc.associated_path(), Some(path.as_path()));
        assert_eq!(doc.language_override(), Some("cpp"));
        assert!(doc.dirty());
        assert!(!path.exists());
        std::fs::write(&path, "existing 猫🙂\r\n").unwrap();
        assert!(capture.prepare().is_err());
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "existing 猫🙂\r\n");
        #[cfg(unix)]
        {
            std::fs::remove_file(&path).unwrap();
            std::os::unix::fs::symlink(root.path().join("missing"), &path).unwrap();
            assert!(capture.prepare().is_err());
        }
    }
    #[test]
    fn uri_opening_rejects_content_injection_nonlocal_authorities_and_invalid_shapes() {
        let root = tempfile::tempdir().unwrap();
        for uri in [
            "file:/a.cpp",
            "untitled:/",
            "untitled:/a.cpp?query",
            "untitled:/a.cpp#fragment",
            "untitled://remote/a.cpp",
        ] {
            assert!(
                Capture::new(
                    context(root.path()),
                    &json!({"uri":uri,"content":"","language":null})
                )
                .is_err(),
                "{uri}"
            );
        }
        assert!(
            Capture::new(
                context(root.path()),
                &json!({"uri":"untitled:/a.cpp","content":"injection","language":null})
            )
            .is_err()
        );
    }
}
