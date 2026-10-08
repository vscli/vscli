//! Debug Adapter Protocol session state. Handles expire whenever execution resumes.
use crate::transport::Process;
use anyhow::{Context, Result, bail};
use serde::Deserialize;
use serde_json::{Value, json};
use std::{
    collections::{BTreeMap, BTreeSet, HashMap, VecDeque},
    path::{Path, PathBuf},
    time::{Duration, Instant},
};

#[derive(Clone)]
pub struct Configuration {
    pub adapter: String,
    pub adapter_args: Vec<String>,
    pub program: Option<PathBuf>,
    pub program_args: Vec<String>,
}
#[derive(Clone, Debug, PartialEq)]
pub enum State {
    Initializing,
    Running,
    Stopped,
    Terminated,
}
#[derive(Clone, Debug, Deserialize)]
pub struct Source {
    pub path: Option<PathBuf>,
}
#[derive(Clone, Debug, Deserialize)]
pub struct Frame {
    pub id: i64,
    pub name: String,
    pub source: Option<Source>,
    pub line: usize,
    pub column: usize,
}
#[derive(Clone, Debug, Deserialize)]
pub struct Scope {
    pub name: String,
    #[serde(rename = "variablesReference")]
    pub reference: i64,
}
#[derive(Clone, Debug, Deserialize)]
pub struct Variable {
    pub name: String,
    pub value: String,
    #[serde(rename = "variablesReference")]
    pub reference: i64,
    #[serde(rename = "type")]
    pub kind: Option<String>,
}
struct Pending {
    command: String,
    epoch: u64,
    view_generation: u64,
    context: String,
    configure: bool,
    started: Instant,
}
pub struct Client {
    process: Process,
    sequence: u64,
    pending: HashMap<u64, Pending>,
    launch: Value,
    epoch: u64,
    view_generation: u64,
    configuring: usize,
    pub capabilities: Value,
    pub state: State,
    pub reason: String,
    pub thread: Option<i64>,
    pub frames: Vec<Frame>,
    pub active_frame: usize,
    pub scopes: Vec<Scope>,
    pub variables: Vec<Variable>,
    pub console: VecDeque<String>,
    pub breakpoints: BTreeMap<PathBuf, BTreeSet<usize>>,
    pub verified: BTreeMap<PathBuf, Vec<(usize, bool)>>,
    pub location: Option<(PathBuf, usize, usize)>,
    started: Instant,
}
impl Client {
    pub fn start(
        config: &Configuration,
        program: &Path,
        root: &Path,
        breakpoints: BTreeMap<PathBuf, BTreeSet<usize>>,
    ) -> Result<Self> {
        let process = Process::start(&config.adapter, &config.adapter_args, root)?;
        let mut client = Self {
            process,
            sequence: 1,
            pending: HashMap::new(),
            epoch: 0,
            view_generation: 0,
            configuring: 0,
            launch: json!({"name":"VSCLI", "request":"launch", "program":program, "cwd":root, "args":config.program_args, "console":"internalConsole", "justMyCode":true, "stopOnEntry":false}),
            capabilities: Value::Null,
            state: State::Initializing,
            reason: "initializing".into(),
            thread: None,
            frames: Vec::new(),
            active_frame: 0,
            scopes: Vec::new(),
            variables: Vec::new(),
            console: VecDeque::new(),
            breakpoints,
            verified: BTreeMap::new(),
            location: None,
            started: Instant::now(),
        };
        client.request("initialize", json!({"clientID":"vscli", "clientName":"VSCLI", "adapterID":"configured", "pathFormat":"path", "linesStartAt1":true, "columnsStartAt1":true, "supportsVariableType":true, "supportsVariablePaging":false, "supportsRunInTerminalRequest":false, "supportsProgressReporting":false, "supportsInvalidatedEvent":false, "supportsMemoryReferences":false, "supportsANSIStyling":false}), String::new(), false)?;
        Ok(client)
    }
    fn request(
        &mut self,
        command: &str,
        arguments: Value,
        context: String,
        configure: bool,
    ) -> Result<()> {
        if self.pending.len() >= 64 {
            bail!("Too many pending debugger requests");
        }
        let sequence = self.sequence;
        self.sequence += 1;
        self.process.send(
            json!({"seq":sequence,"type":"request","command":command,"arguments":arguments}),
        )?;
        self.pending.insert(
            sequence,
            Pending {
                command: command.into(),
                epoch: self.epoch,
                view_generation: self.view_generation,
                context,
                configure,
                started: Instant::now(),
            },
        );
        Ok(())
    }
    fn output(&mut self, text: &str) {
        for line in text.lines() {
            self.console.push_back(line.chars().take(4000).collect());
        }
        while self.console.len() > 2000 {
            self.console.pop_front();
        }
    }
    pub fn set_breakpoints(&mut self, path: PathBuf, lines: BTreeSet<usize>) -> Result<()> {
        self.breakpoints.insert(path.clone(), lines);
        if self.state != State::Initializing && self.state != State::Terminated {
            self.send_breakpoints(&path, false)?;
        }
        Ok(())
    }
    fn send_breakpoints(&mut self, path: &Path, configure: bool) -> Result<()> {
        let breakpoints: Vec<_> = self
            .breakpoints
            .get(path)
            .into_iter()
            .flatten()
            .map(|line| json!({"line":line}))
            .collect();
        if configure {
            self.configuring += 1;
        }
        self.request(
            "setBreakpoints",
            json!({"source":{"path":path},"breakpoints":breakpoints,"sourceModified":false}),
            path.to_string_lossy().into_owned(),
            configure,
        )
    }
    fn configure_done(&mut self) -> Result<()> {
        if self.capabilities["supportsConfigurationDoneRequest"] == true {
            self.request("configurationDone", json!({}), String::new(), false)?;
        }
        Ok(())
    }
    pub fn resume(&mut self, command: &str) -> Result<()> {
        if self.state != State::Stopped {
            bail!("Debugger is not paused");
        }
        let thread = self.thread.context("No stopped thread selected")?;
        self.epoch += 1;
        self.request(command, json!({"threadId":thread}), String::new(), false)?;
        self.state = State::Running;
        self.reason = command.into();
        self.frames.clear();
        self.scopes.clear();
        self.variables.clear();
        self.location = None;
        Ok(())
    }
    pub fn pause(&mut self) -> Result<()> {
        if let Some(thread) = self.thread {
            self.request("pause", json!({"threadId":thread}), String::new(), false)
        } else {
            self.request("threads", json!({}), "pause".into(), false)
        }
    }
    pub fn stop(&mut self) -> Result<()> {
        self.request(
            "disconnect",
            json!({"restart":false,"terminateDebuggee":true}),
            String::new(),
            false,
        )
    }
    pub fn select_frame(&mut self, index: usize) -> Result<()> {
        if self.state != State::Stopped {
            bail!("Debugger is not paused");
        }
        let frame = self
            .frames
            .get(index)
            .context("No stack frame selected")?
            .clone();
        self.active_frame = index;
        self.view_generation += 1;
        self.scopes.clear();
        self.variables.clear();
        self.location = frame
            .source
            .and_then(|s| s.path)
            .map(|p| (p, frame.line, frame.column));
        self.request(
            "scopes",
            json!({"frameId":frame.id}),
            frame.id.to_string(),
            false,
        )
    }
    pub fn expand(&mut self, reference: i64) -> Result<()> {
        if self.state != State::Stopped {
            bail!("Debugger is not paused");
        }
        if reference <= 0 {
            return Ok(());
        }
        self.view_generation += 1;
        self.variables.clear();
        self.request(
            "variables",
            json!({"variablesReference":reference}),
            String::new(),
            false,
        )
    }
    pub fn evaluate(&mut self, expression: &str) -> Result<()> {
        if self.state != State::Stopped {
            bail!("Pause execution before evaluating an expression");
        }
        let frame = self
            .frames
            .get(self.active_frame)
            .context("No frame selected")?;
        self.request(
            "evaluate",
            json!({"expression":expression,"frameId":frame.id,"context":"repl"}),
            expression.into(),
            false,
        )
    }
    pub fn poll(&mut self) -> Result<bool> {
        let mut changed = false;
        if self.state == State::Initializing && self.started.elapsed() > Duration::from_secs(30) {
            bail!("Debugger initialization timed out");
        }
        for id in self
            .pending
            .iter()
            .filter(|(_, p)| p.started.elapsed() > Duration::from_secs(30))
            .map(|(id, _)| *id)
            .collect::<Vec<_>>()
        {
            let request = self.pending.remove(&id).unwrap();
            self.output(&format!("Debugger request timed out: {}", request.command));
            self.failed_request(&request)?;
            if request.configure {
                bail!("Debugger configuration timed out: {}", request.command);
            }
            changed = true;
        }
        for _ in 0..32 {
            let Some(message) = self.process.receive()? else {
                break;
            };
            changed = true;
            match message["type"].as_str() {
                Some("event") => {
                    self.event(message["event"].as_str().unwrap_or(""), &message["body"])?
                }
                Some("response") => {
                    let Some(sequence) = message["request_seq"].as_u64() else {
                        continue;
                    };
                    let Some(pending) = self.pending.remove(&sequence) else {
                        continue;
                    };
                    if message["success"] != true {
                        self.output(&format!(
                            "{}: {}",
                            pending.command,
                            message["message"].as_str().unwrap_or("failed")
                        ));
                        self.failed_request(&pending)?;
                    } else {
                        self.response(&pending, &message["body"])?;
                    }
                    if pending.configure {
                        self.configuring = self.configuring.saturating_sub(1);
                        if self.configuring == 0 {
                            self.configure_done()?;
                        }
                    }
                }
                Some("request") => {
                    let sequence = self.sequence;
                    self.sequence += 1;
                    self.process.send(json!({"seq":sequence,"type":"response","request_seq":message["seq"],"command":message["command"],"success":false,"message":"This client does not support reverse requests"}))?;
                }
                _ => {}
            }
        }
        Ok(changed)
    }
    fn event(&mut self, event: &str, body: &Value) -> Result<()> {
        match event {
            "initialized" => {
                self.configuring = 0;
                for path in self.breakpoints.keys().cloned().collect::<Vec<_>>() {
                    self.send_breakpoints(&path, true)?;
                }
                let filters: Vec<_> = self.capabilities["exceptionBreakpointFilters"]
                    .as_array()
                    .into_iter()
                    .flatten()
                    .filter(|f| f["default"] == true)
                    .filter_map(|f| f["filter"].as_str())
                    .collect();
                self.configuring += 1;
                self.request(
                    "setExceptionBreakpoints",
                    json!({"filters":filters}),
                    String::new(),
                    true,
                )?;
            }
            "stopped" => {
                self.epoch += 1;
                self.state = State::Stopped;
                self.reason = body["reason"].as_str().unwrap_or("paused").into();
                self.thread = body["threadId"].as_i64();
                self.frames.clear();
                self.scopes.clear();
                self.variables.clear();
                if let Some(thread) = self.thread {
                    self.request(
                        "stackTrace",
                        json!({"threadId":thread,"startFrame":0,"levels":100}),
                        String::new(),
                        false,
                    )?;
                } else {
                    self.request("threads", json!({}), String::new(), false)?;
                }
            }
            "continued" => {
                self.epoch += 1;
                self.state = State::Running;
                self.reason = "running".into();
                self.frames.clear();
                self.scopes.clear();
                self.variables.clear();
                self.location = None;
            }
            "terminated" | "exited" => {
                self.epoch += 1;
                self.state = State::Terminated;
                self.reason = "terminated".into();
                self.location = None;
            }
            "output" => self.output(body["output"].as_str().unwrap_or("")),
            "thread" if self.thread.is_none() => {
                self.thread = body["threadId"].as_i64();
            }
            _ => {}
        }
        Ok(())
    }
    fn response(&mut self, pending: &Pending, body: &Value) -> Result<()> {
        if matches!(
            pending.command.as_str(),
            "stackTrace" | "scopes" | "variables" | "evaluate"
        ) && (pending.epoch != self.epoch || self.state != State::Stopped)
        {
            return Ok(());
        }
        if matches!(pending.command.as_str(), "scopes" | "variables")
            && pending.view_generation != self.view_generation
        {
            return Ok(());
        }
        match pending.command.as_str() {
            "initialize" => {
                self.capabilities = body.clone();
                self.request("launch", self.launch.clone(), String::new(), false)?;
            }
            "launch" => {
                if self.state == State::Initializing {
                    self.state = State::Running;
                    self.reason = "running".into();
                }
            }
            "setBreakpoints" => {
                let breakpoints = body["breakpoints"]
                    .as_array()
                    .into_iter()
                    .flatten()
                    .filter_map(|b| {
                        b["line"]
                            .as_u64()
                            .map(|line| (line as usize, b["verified"] == true))
                    })
                    .collect();
                self.verified
                    .insert(PathBuf::from(&pending.context), breakpoints);
            }
            "threads" => {
                self.thread = body["threads"]
                    .as_array()
                    .and_then(|a| a.first())
                    .and_then(|t| t["id"].as_i64());
                if pending.context == "pause" {
                    if self.thread.is_some() {
                        self.pause()?;
                    }
                } else if let Some(thread) = self.thread {
                    self.request(
                        "stackTrace",
                        json!({"threadId":thread,"startFrame":0,"levels":100}),
                        String::new(),
                        false,
                    )?;
                }
            }
            "stackTrace" => {
                self.frames = serde_json::from_value::<Vec<Frame>>(body["stackFrames"].clone())?
                    .into_iter()
                    .take(100)
                    .collect();
                for frame in &mut self.frames {
                    if let Some(path) = frame.source.as_mut().and_then(|s| s.path.as_mut()) {
                        *path =
                            crate::document::absolute_path(path).unwrap_or_else(|_| path.clone());
                    }
                }
                if !self.frames.is_empty() {
                    self.select_frame(0)?;
                }
            }
            "scopes" => {
                if self
                    .frames
                    .get(self.active_frame)
                    .is_none_or(|f| f.id.to_string() != pending.context)
                {
                    return Ok(());
                }
                self.scopes = serde_json::from_value::<Vec<Scope>>(body["scopes"].clone())?
                    .into_iter()
                    .take(100)
                    .collect();
                if let Some(scope) = self.scopes.first() {
                    self.expand(scope.reference)?;
                }
            }
            "variables" => {
                self.variables = serde_json::from_value::<Vec<Variable>>(body["variables"].clone())?
                    .into_iter()
                    .take(1000)
                    .collect()
            }
            "evaluate" => self.output(&format!(
                "{} = {}",
                pending.context,
                body["result"].as_str().unwrap_or("")
            )),
            "disconnect" => {
                self.state = State::Terminated;
                self.reason = "disconnected".into();
            }
            _ => {}
        }
        Ok(())
    }
    fn failed_request(&mut self, pending: &Pending) -> Result<()> {
        if matches!(
            pending.command.as_str(),
            "initialize" | "launch" | "configurationDone"
        ) {
            self.state = State::Terminated;
        } else if matches!(
            pending.command.as_str(),
            "continue" | "next" | "stepIn" | "stepOut"
        ) && pending.epoch == self.epoch
            && self.state == State::Running
        {
            // A rejected resume leaves the target paused. Fetch fresh handles rather
            // than exposing handles that may have expired during the request.
            self.event(
                "stopped",
                &json!({"threadId":self.thread,"reason":"resume rejected"}),
            )?;
        }
        Ok(())
    }
}

impl Drop for Client {
    fn drop(&mut self) {
        // Give adapters a bounded opportunity to clean up their debuggee before
        // the transport kills/reaps the adapter. This also runs on editor errors.
        let _ = self
            .process
            .send(json!({"seq":self.sequence,"type":"request",
            "command":"disconnect","arguments":{"restart":false,"terminateDebuggee":true}}));
        let started = Instant::now();
        while started.elapsed() < Duration::from_millis(500) {
            if self.process.exited() {
                break;
            }
            // Drain output so a chatty adapter cannot block its own shutdown.
            if self.process.receive().is_err() {
                break;
            }
            std::thread::sleep(Duration::from_millis(5));
        }
    }
}
