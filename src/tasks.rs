use anyhow::{Context, Result, bail};
use portable_pty::CommandBuilder;
use serde_json::Value;
use std::path::{Path, PathBuf};

#[derive(Clone)]
pub struct Task {
    pub label: String,
    pub definition: Value,
}
pub struct PreparedTask {
    pub label: String,
    pub command: CommandBuilder,
    pub preview: String,
    pub notice: String,
}
pub struct Variables<'a> {
    pub root: &'a Path,
    pub file: Option<&'a Path>,
    pub line: usize,
    pub selected: &'a str,
}
pub fn load(root: &Path) -> Result<Vec<Task>> {
    let path = root.join(".vscode/tasks.json");
    let raw = std::fs::read_to_string(&path).with_context(|| {
        format!(
            "Cannot read {}; create a VS Code tasks.json with shell/process tasks",
            path.display()
        )
    })?;
    if raw.len() > 1024 * 1024 {
        bail!("tasks.json exceeds 1 MiB");
    }
    let config: Value = json5::from_str(&raw).context("Invalid tasks.json")?;
    if config["version"] != "2.0.0" {
        bail!("Only tasks.json version 2.0.0 is supported");
    }
    let definitions = config["tasks"].as_array().context("Missing tasks array")?;
    let platform = if cfg!(windows) {
        "windows"
    } else if cfg!(target_os = "macos") {
        "osx"
    } else {
        "linux"
    };
    let mut tasks = Vec::new();
    for definition in definitions {
        let mut definition = definition.clone();
        if let Some(overrides) = definition[platform].as_object().cloned() {
            definition
                .as_object_mut()
                .context("Task must be an object")?
                .extend(overrides);
        }
        let label = definition["label"]
            .as_str()
            .context("Task requires a label")?
            .to_owned();
        tasks.push(Task { label, definition });
    }
    Ok(tasks)
}
pub fn is_build(task: &Task) -> bool {
    task.definition["group"] == "build" || task.definition["group"]["kind"] == "build"
}
pub fn is_default_build(task: &Task) -> bool {
    is_build(task) && task.definition["group"]["isDefault"] == true
}
fn expand(text: &str, variables: &Variables<'_>) -> Result<String> {
    let mut rest = text;
    let mut output = String::new();
    while let Some(start) = rest.find("${") {
        output.push_str(&rest[..start]);
        let tail = &rest[start + 2..];
        let end = tail.find('}').context("Unclosed task variable")?;
        let key = &tail[..end];
        let file = || {
            variables
                .file
                .context("This task variable requires a saved file path")
        };
        let value = match key {
            "workspaceFolder" | "cwd" => variables.root.to_string_lossy().into_owned(),
            "workspaceFolderBasename" => variables
                .root
                .file_name()
                .unwrap_or_default()
                .to_string_lossy()
                .into_owned(),
            "file" => file()?.to_string_lossy().into_owned(),
            "relativeFile" => file()?
                .strip_prefix(variables.root)
                .unwrap_or(file()?)
                .to_string_lossy()
                .into_owned(),
            "fileBasename" => file()?
                .file_name()
                .unwrap_or_default()
                .to_string_lossy()
                .into_owned(),
            "fileBasenameNoExtension" => file()?
                .file_stem()
                .unwrap_or_default()
                .to_string_lossy()
                .into_owned(),
            "fileDirname" => file()?
                .parent()
                .unwrap_or(variables.root)
                .to_string_lossy()
                .into_owned(),
            "fileExtname" => file()?
                .extension()
                .map_or(String::new(), |s| format!(".{}", s.to_string_lossy())),
            "lineNumber" => variables.line.to_string(),
            "selectedText" => variables.selected.to_owned(),
            "pathSeparator" | "/" => std::path::MAIN_SEPARATOR.to_string(),
            _ if key.starts_with("env:") => std::env::var(&key[4..]).with_context(|| {
                format!("Task environment variable is unavailable: {}", &key[4..])
            })?,
            _ => bail!("Unsupported task variable: {key}"),
        };
        output.push_str(&value);
        rest = &tail[end + 1..];
    }
    output.push_str(rest);
    Ok(output)
}
fn quote_posix(text: &str) -> String {
    format!("'{}'", text.replace('\'', "'\\''"))
}
pub fn prepare(task: &Task, variables: &Variables<'_>) -> Result<PreparedTask> {
    let definition = &task.definition;
    if definition.get("dependsOn").is_some() {
        bail!("Task dependencies are not implemented yet");
    }
    if definition["isBackground"] == true {
        bail!("Background task readiness is not implemented yet");
    }
    let kind = definition["type"]
        .as_str()
        .context("Task requires type: process or shell")?;
    let executable = expand(
        definition["command"]
            .as_str()
            .context("Task command must be a string")?,
        variables,
    )?;
    let args = definition
        .get("args")
        .map(|v| -> Result<Vec<String>> {
            v.as_array()
                .context("Task args must be an array")?
                .iter()
                .map(|v| {
                    expand(
                        v.as_str()
                            .context("Only string task arguments are supported")?,
                        variables,
                    )
                })
                .collect()
        })
        .transpose()?
        .unwrap_or_default();
    let options = &definition["options"];
    let cwd = if let Some(cwd) = options["cwd"].as_str() {
        let path = PathBuf::from(expand(cwd, variables)?);
        if path.is_absolute() {
            path
        } else {
            variables.root.join(path)
        }
    } else {
        variables.root.to_path_buf()
    };
    let (mut command, preview) = match kind {
        "process" => {
            let mut command = CommandBuilder::new(&executable);
            command.args(&args);
            (command, format!("{executable:?} {args:?}"))
        }
        "shell" => {
            if cfg!(windows) {
                bail!("Windows shell task quoting is not qualified yet; use a process task");
            }
            let shell = options["shell"]["executable"].as_str().unwrap_or("/bin/sh");
            let mut command = CommandBuilder::new(expand(shell, variables)?);
            if let Some(shell_args) = options["shell"]["args"].as_array() {
                for argument in shell_args {
                    command.arg(expand(
                        argument
                            .as_str()
                            .context("Shell arguments must be strings")?,
                        variables,
                    )?);
                }
            } else {
                command.arg("-c");
            }
            // With arguments, quote the executable and each argument separately.
            // Without arguments, VS Code's command string may itself be a pipeline.
            let line = if args.is_empty() {
                executable
            } else {
                std::iter::once(quote_posix(&executable))
                    .chain(args.iter().map(|a| quote_posix(a)))
                    .collect::<Vec<_>>()
                    .join(" ")
            };
            command.arg(&line);
            (command, line)
        }
        _ => bail!("Unsupported task provider: {kind}; extensions are not active"),
    };
    command.cwd(&cwd);
    let mut env_names = Vec::new();
    if let Some(environment) = options.get("env") {
        for (key, value) in environment
            .as_object()
            .context("Task environment must be an object")?
        {
            if value.is_null() {
                command.env_remove(key);
            } else {
                command.env(
                    key,
                    expand(
                        value
                            .as_str()
                            .context("Task environment values must be strings or null")?,
                        variables,
                    )?,
                );
            }
            env_names.push(key.as_str());
        }
    }
    let matcher = definition
        .get("problemMatcher")
        .is_some_and(|v| !v.is_null() && v.as_array().is_none_or(|a| !a.is_empty()));
    Ok(PreparedTask {
        label: task.label.clone(),
        command,
        preview: format!(
            "Task: {}\nDirectory: {}\nCommand: {}\nEnvironment overrides: {}",
            task.label,
            cwd.display(),
            preview,
            env_names.join(", ")
        ),
        notice: if matcher {
            "Task output is shown; problem matchers are not implemented yet".into()
        } else {
            String::new()
        },
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn task_arguments_are_not_shell_code_and_variables_preserve_spaces() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("my file.rs");
        let vars = Variables {
            root: dir.path(),
            file: Some(&file),
            line: 4,
            selected: "selection",
        };
        let task = Task {
            label: "example".into(),
            definition: serde_json::json!({"type":"shell", "command":"printf", "args":["%s", "${fileBasename}", "$(touch unwanted)", "it's literal"]}),
        };
        if cfg!(windows) {
            assert!(
                prepare(&task, &vars)
                    .err()
                    .unwrap()
                    .to_string()
                    .contains("Windows shell task quoting")
            );
        } else {
            let task = prepare(&task, &vars).unwrap();
            assert!(task.preview.contains("'$(touch unwanted)'"));
            assert!(task.preview.contains("'it'\\''s literal'"));
            assert!(task.preview.contains("'my file.rs'"));
        }
        let process = Task {
            label: "process".into(),
            definition: serde_json::json!({"type":"process", "command":"echo", "args":["${fileBasename}", "$(touch unwanted)"]}),
        };
        let process = prepare(&process, &vars).unwrap();
        assert_eq!(
            process.command.get_argv()[1..],
            ["my file.rs", "$(touch unwanted)"]
        );
        assert!(expand("${command:arbitrary}", &vars).is_err());
    }
    #[test]
    fn jsonc_platform_override_and_default_build_selection() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::create_dir(dir.path().join(".vscode")).unwrap();
        std::fs::write(dir.path().join(".vscode/tasks.json"), r#"{version:'2.0.0',tasks:[{label:'build',type:'process',command:'echo',group:{kind:'build',isDefault:true}, // comment
        args:['ok'],}],}"#).unwrap();
        let tasks = load(dir.path()).unwrap();
        assert_eq!(tasks.len(), 1);
        assert!(is_default_build(&tasks[0]));
    }
}
