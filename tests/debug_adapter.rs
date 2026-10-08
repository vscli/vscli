use serde_json::Value;
use std::time::{Duration, Instant};
use vscli::{
    app::App,
    debug::{Configuration, State},
    keys::Profile,
};

fn until(app: &mut App, predicate: impl Fn(&App) -> bool) {
    let started = Instant::now();
    loop {
        app.poll();
        if predicate(app) {
            return;
        }
        assert!(
            started.elapsed() < Duration::from_secs(20),
            "{}; {:?}",
            app.message,
            app.debugger.as_ref().map(|d| (&d.reason, &d.console))
        );
        std::thread::sleep(Duration::from_millis(5));
    }
}
#[test]
fn protocol_reordered_variables_rejected_step_and_shutdown() {
    let dir = tempfile::tempdir().unwrap();
    let program = dir.path().join("program.py");
    std::fs::write(&program, "one\ntwo\nthree\n").unwrap();
    let mut app = App::new(dir.path().into(), Profile::Linux);
    app.open(&program).unwrap();
    app.debug_configuration = Some(Configuration {
        adapter: if cfg!(windows) { "python" } else { "python3" }.into(),
        adapter_args: vec![format!(
            "{}/tests/fixtures/debug_adapter.py",
            env!("CARGO_MANIFEST_DIR")
        )],
        program: Some(program.clone()),
        program_args: Vec::new(),
    });
    app.execute("workbench.action.debug.start", Value::Null);
    until(&mut app, |a| {
        a.debugger.as_ref().is_some_and(|d| !d.variables.is_empty())
    });
    assert_eq!(app.doc().row(), 1);
    assert_eq!(
        app.debugger.as_ref().unwrap().frames[0]
            .source
            .as_ref()
            .unwrap()
            .path,
        app.doc().path
    );
    let debugger = app.debugger.as_mut().unwrap();
    debugger.expand(11).unwrap();
    debugger.expand(12).unwrap();
    until(&mut app, |a| {
        a.debugger
            .as_ref()
            .unwrap()
            .variables
            .first()
            .is_some_and(|v| v.name == "value12")
    });
    // Both responses are emitted by the fixture. Pump any response left in transit.
    for _ in 0..20 {
        app.poll();
        std::thread::sleep(Duration::from_millis(5));
    }
    assert_eq!(app.debugger.as_ref().unwrap().variables[0].name, "value12");
    app.execute("workbench.action.debug.stepInto", Value::Null);
    until(&mut app, |a| {
        a.debugger.as_ref().unwrap().reason == "resume rejected"
            && !a.debugger.as_ref().unwrap().variables.is_empty()
    });
    assert_eq!(app.debugger.as_ref().unwrap().state, State::Stopped);
    app.execute("workbench.action.debug.stepOver", Value::Null);
    until(&mut app, |a| {
        a.doc().row() == 2 && !a.debugger.as_ref().unwrap().variables.is_empty()
    });
    drop(app);
    assert_eq!(
        std::fs::read_to_string(program.with_extension("py.disconnected")).unwrap(),
        "clean shutdown"
    );
}

#[test]
#[ignore = "requires debugpy installed; real-adapter qualification"]
fn real_debugpy_breakpoint_variables_step_evaluate_continue() {
    let dir = tempfile::tempdir().unwrap();
    let program = dir.path().join("program.py");
    std::fs::write(
        &program,
        "def add(a, b):\n    result = a + b\n    return result\nx = 4\ny = add(x, 3)\nprint(y)\n",
    )
    .unwrap();
    let mut app = App::new(dir.path().into(), Profile::Linux);
    app.open(&program).unwrap();
    let position = app.doc().line_start(4);
    app.doc_mut().move_to(position, false);
    app.execute("editor.debug.action.toggleBreakpoint", Value::Null);
    app.debug_configuration = Some(Configuration {
        adapter: std::env::var("VSCLI_TEST_PYTHON")
            .unwrap_or_else(|_| if cfg!(windows) { "python" } else { "python3" }.into()),
        adapter_args: vec!["-m".into(), "debugpy.adapter".into()],
        program: Some(program.clone()),
        program_args: Vec::new(),
    });
    app.execute("workbench.action.debug.start", Value::Null);
    until(&mut app, |a| {
        a.debugger
            .as_ref()
            .is_some_and(|d| d.state == State::Stopped && d.variables.iter().any(|v| v.name == "x"))
    });
    assert_eq!(app.doc().row(), 4);
    assert!(
        app.debugger
            .as_ref()
            .unwrap()
            .variables
            .iter()
            .any(|v| v.name == "x" && v.value == "4")
    );
    app.execute("workbench.action.debug.stepOver", Value::Null);
    until(&mut app, |a| {
        a.debugger
            .as_ref()
            .is_some_and(|d| d.state == State::Stopped && d.variables.iter().any(|v| v.name == "y"))
    });
    assert_eq!(app.doc().row(), 5);
    app.debugger.as_mut().unwrap().evaluate("x + y").unwrap();
    until(&mut app, |a| {
        a.debugger
            .as_ref()
            .unwrap()
            .console
            .iter()
            .any(|s| s == "x + y = 11")
    });
    app.execute("workbench.action.debug.continue", Value::Null);
    until(&mut app, |a| {
        a.debugger
            .as_ref()
            .is_some_and(|d| d.state == State::Terminated)
    });
    assert!(
        app.debugger
            .as_ref()
            .unwrap()
            .console
            .iter()
            .any(|s| s == "7")
    );
}
