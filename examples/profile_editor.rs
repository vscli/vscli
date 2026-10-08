//! Native event/poll/render phase measurements, excluding PTY/terminal costs.
//! cargo run --release --example profile_editor -- --iterations 2000
use anyhow::Result;
use clap::Parser;
use crossterm::event::{Event, KeyCode, KeyEvent, KeyModifiers};
use ratatui::{Terminal, backend::TestBackend};
use serde_json::{Value, json};
use std::{fs, time::Instant};
use vscli::{app::App, keys::Profile, ui};

#[derive(Parser)]
struct Args {
    #[arg(long, default_value_t = 2000, value_parser = clap::value_parser!(u32).range(1..))]
    iterations: u32,
    #[arg(long, default_value_t = 10000, value_parser = clap::value_parser!(u32).range(1..100000))]
    rows: u32,
    #[arg(long)]
    unicode: bool,
}

fn distribution(mut samples: Vec<f64>) -> Value {
    samples.sort_by(f64::total_cmp);
    let at = |percent: usize| samples[(samples.len() * percent).div_ceil(100) - 1];
    json!({"n":samples.len(), "unit":"microseconds", "p50":at(50),
        "p95":at(95), "p99":at(99), "max":samples.last()})
}

fn main() -> Result<()> {
    let args = Args::parse();
    let directory = tempfile::tempdir()?;
    let source = directory.path().join("fixture.txt");
    let line = if args.unicode {
        "e\u{301}\t猫🙂 Αλφάβητο café 👩\u{200d}💻 Unicode movement and rendering\n"
    } else {
        "ABCDEFGHIJKLMNOPQRSTUVWXYZ0123456789 ABCDEFGHIJKLMNOPQRSTUVWXYZ0123456789\n"
    };
    fs::write(&source, line.repeat(args.rows as usize))?;
    let mut app = App::new(directory.path().to_path_buf(), Profile::Linux);
    app.open(&source)?;
    let mut terminal = Terminal::new(TestBackend::new(120, 40))?;
    let mut events = Vec::new();
    let mut polls = Vec::new();
    let mut renders = Vec::new();
    for step in 0..args.iterations + 100 {
        // Undo keeps text, viewport and edit size constant between samples.
        terminal.draw(|frame| ui::draw(frame, &mut app))?;
        let start = Instant::now();
        app.event(Event::Key(KeyEvent::new(
            KeyCode::Char('x'),
            KeyModifiers::NONE,
        )));
        let event_us = start.elapsed().as_secs_f64() * 1e6;
        let start = Instant::now();
        app.poll();
        let poll_us = start.elapsed().as_secs_f64() * 1e6;
        let start = Instant::now();
        terminal.draw(|frame| ui::draw(frame, &mut app))?;
        let render_us = start.elapsed().as_secs_f64() * 1e6;
        if step >= 100 {
            events.push(event_us);
            polls.push(poll_us);
            renders.push(render_us);
        }
        app.doc_mut().undo();
    }
    assert_eq!(app.doc().text.to_string(), line.repeat(args.rows as usize));
    println!(
        "{}",
        serde_json::to_string_pretty(&json!({
            "schema":1, "unicode":args.unicode, "rows":args.rows,
            "terminal_cells":[120,40], "warmup_iterations":100,
            "boundary":"native App event, poll, and Ratatui TestBackend draw; no PTY or display",
            "event":distribution(events), "poll":distribution(polls),
            "render":distribution(renders),
        }))?
    );
    Ok(())
}
