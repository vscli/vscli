//! Cached native welcome artwork. Decoding and protocol encoding stay on one worker.
use anyhow::{Result, bail};
use image::{DynamicImage, ImageFormat, ImageReader};
use ratatui::{
    Frame,
    layout::{Rect, Size},
    style::Color,
};
use ratatui_image::{
    Image,
    protocol::{Protocol, halfblocks::Halfblocks, kitty::Kitty},
};
use std::{
    io::Cursor,
    sync::{
        atomic::{AtomicBool, Ordering},
        mpsc::{Receiver, SyncSender, sync_channel},
    },
    time::{Duration, Instant},
};

const PNG: &[u8] = include_bytes!("../assets/brand/vscli-mark.png");
const MAX_ENCODED: usize = 64 * 1024;
const IMAGE_ID: u32 = 0x5653434c;
const DELETE: &str = "\x1b_Ga=d,d=I,i=1448297292,q=2\x1b\\";
static UPLOADED: AtomicBool = AtomicBool::new(false);
const DEADLINE: Duration = Duration::from_secs(2);

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Mode {
    Kitty,
    Cells,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Key {
    width: u16,
    height: u16,
    background: [u8; 3],
    mode: Mode,
    generation: u64,
}
impl Key {
    fn valid(self) -> bool {
        (4..=28).contains(&self.width) && (2..=14).contains(&self.height)
    }
}
struct Worker {
    sender: SyncSender<Key>,
    receiver: Receiver<(Key, Result<Protocol>)>,
    pending: Option<(Key, Instant)>,
}
pub struct State {
    mode: Mode,
    desired: Option<Key>,
    ready: Option<(Key, Protocol)>,
    failed: Option<Key>,
    worker: Option<Worker>,
    uploaded: Option<Key>,
    generation: u64,
}
impl Default for State {
    fn default() -> Self {
        Self::new(
            colors_enabled(std::env::var("NO_COLOR").ok().as_deref())
                && kitty_environment(
                    std::env::var("TERM").ok().as_deref(),
                    std::env::var("KITTY_WINDOW_ID").ok().as_deref(),
                    std::env::var_os("TMUX").is_some() || std::env::var_os("STY").is_some(),
                ),
        )
    }
}
fn colors_enabled(no_color: Option<&str>) -> bool {
    no_color.is_none_or(str::is_empty)
}
fn kitty_environment(term: Option<&str>, window: Option<&str>, multiplexed: bool) -> bool {
    !multiplexed
        && term == Some("xterm-kitty")
        && window.is_some_and(|v| !v.is_empty() && v.bytes().all(|c| c.is_ascii_digit()))
}
impl State {
    fn new(kitty: bool) -> Self {
        Self {
            mode: if kitty { Mode::Kitty } else { Mode::Cells },
            desired: None,
            ready: None,
            failed: None,
            worker: None,
            uploaded: None,
            generation: 0,
        }
    }
    pub fn resize(&mut self) {
        self.generation = self.generation.wrapping_add(1);
        self.desired = None;
        self.ready = None;
    }
    pub fn begin_frame(&mut self) {
        self.desired = None;
    }
    /// Rendering only reads encoded state and records the latest bounded request.
    pub fn render(&mut self, frame: &mut Frame, area: Rect, background: Color) {
        let key = Key {
            width: area.width,
            height: area.height,
            background: rgb(background),
            mode: self.mode,
            generation: self.generation,
        };
        if !key.valid() {
            return;
        }
        self.desired = Some(key);
        let Some((ready, protocol)) = &self.ready else {
            return;
        };
        if *ready != key {
            return;
        }
        frame.render_widget(Image::new(protocol), area);
        if matches!(protocol, Protocol::Kitty(_)) {
            if self.uploaded.is_some_and(|old| old != key) {
                // The fixed image ID is reused. Delete the prior payload immediately
                // before transmitting its replacement, preserving bounded terminal storage.
                if let Some(cell) = frame.buffer_mut().cell_mut((area.x, area.y)) {
                    cell.set_symbol(&format!("{DELETE}{}", cell.symbol()));
                }
            }
            self.uploaded = Some(key);
            UPLOADED.store(true, Ordering::Relaxed);
        }
    }
    pub fn poll(&mut self) -> bool {
        let mut changed = false;
        if let Some(worker) = &mut self.worker {
            if let Ok((key, result)) = worker.receiver.try_recv() {
                let timely = worker
                    .pending
                    .take()
                    .is_some_and(|(_, started)| started.elapsed() <= DEADLINE);
                if Some(key) == self.desired && timely {
                    match result {
                        Ok(protocol) => {
                            self.ready = Some((key, protocol));
                            self.failed = None;
                        }
                        Err(_) => self.failed = Some(key),
                    }
                    changed = true;
                } else if !timely {
                    self.failed = Some(key);
                }
            }
            if let Some((key, started)) = worker.pending
                && started.elapsed() > DEADLINE
            {
                self.failed = Some(key); // Keep the occupied slot until its real completion.
            }
        }
        let Some(key) = self.desired else {
            return changed;
        };
        if self.failed == Some(key) || self.ready.as_ref().is_some_and(|(ready, _)| *ready == key) {
            return changed;
        }
        let worker = self.worker.get_or_insert_with(Worker::start);
        if worker.pending.is_none() && worker.sender.try_send(key).is_ok() {
            worker.pending = Some((key, Instant::now()));
        }
        changed
    }
}
impl Worker {
    fn start() -> Self {
        let (sender, requests) = sync_channel::<Key>(1);
        let (results, receiver) = sync_channel(1);
        std::thread::spawn(move || {
            let decoded = decode(PNG);
            while let Ok(key) = requests.recv() {
                let result = decoded
                    .as_ref()
                    .map_err(|e| anyhow::anyhow!("{e:#}"))
                    .and_then(|image| encode(image, key, cell_pixels()));
                if results.send((key, result)).is_err() {
                    break;
                }
            }
        });
        Self {
            sender,
            receiver,
            pending: None,
        }
    }
}
fn decode(bytes: &[u8]) -> Result<DynamicImage> {
    if bytes.len() > MAX_ENCODED {
        bail!("Brand image exceeds 64 KiB");
    }
    let mut reader = ImageReader::with_format(Cursor::new(bytes), ImageFormat::Png);
    let mut limits = image::Limits::default();
    limits.max_image_width = Some(512);
    limits.max_image_height = Some(512);
    limits.max_alloc = Some(4 * 1024 * 1024);
    reader.limits(limits);
    Ok(reader.decode()?)
}
fn cell_pixels() -> Option<(u16, u16)> {
    let size = crossterm::terminal::window_size().ok()?;
    if size.columns == 0 || size.rows == 0 {
        return None;
    }
    let cell = (size.width / size.columns, size.height / size.rows);
    ((1..=64).contains(&cell.0) && (1..=128).contains(&cell.1)).then_some(cell)
}
fn encode(source: &DynamicImage, key: Key, cell: Option<(u16, u16)>) -> Result<Protocol> {
    if !key.valid() {
        bail!("Brand cell dimensions exceed 28 by 14");
    }
    let size = Size::new(key.width, key.height);
    if key.mode == Mode::Kitty
        && let Some((cell_width, cell_height)) = cell
    {
        let width = u32::from(key.width) * u32::from(cell_width);
        let height = u32::from(key.height) * u32::from(cell_height);
        if width <= 1024 && height <= 1024 {
            let resized = source.resize(width, height, image::imageops::FilterType::Triangle);
            let mut canvas = image::RgbaImage::new(width, height);
            image::imageops::overlay(
                &mut canvas,
                &resized,
                i64::from((width - resized.width()) / 2),
                i64::from((height - resized.height()) / 2),
            );
            return Ok(Protocol::Kitty(Kitty::new(
                DynamicImage::ImageRgba8(canvas),
                size,
                IMAGE_ID,
                false,
                true,
            )?));
        }
    }
    let mut image = source
        .resize_exact(
            u32::from(key.width),
            u32::from(key.height) * 2,
            image::imageops::FilterType::Nearest,
        )
        .to_rgba8();
    for pixel in image.pixels_mut() {
        let alpha = u32::from(pixel[3]);
        for (value, background) in pixel.0[..3].iter_mut().zip(key.background) {
            *value =
                ((u32::from(*value) * alpha + u32::from(background) * (255 - alpha)) / 255) as u8;
        }
        pixel[3] = 255;
    }
    Ok(Protocol::Halfblocks(Halfblocks::new(
        DynamicImage::ImageRgba8(image),
        size,
    )?))
}
fn rgb(color: Color) -> [u8; 3] {
    match color {
        Color::Rgb(r, g, b) => [r, g, b],
        Color::White => [255; 3],
        Color::Black => [0; 3],
        _ => [20, 23, 30],
    }
}
/// Called by terminal restoration, including error/signal unwinding.
pub fn terminal_cleanup() -> Option<&'static str> {
    UPLOADED.swap(false, Ordering::Relaxed).then_some(DELETE)
}

#[cfg(test)]
mod tests {
    use super::*;
    use ratatui::{Terminal, backend::TestBackend};
    fn key(mode: Mode) -> Key {
        Key {
            width: 20,
            height: 10,
            background: [20, 24, 33],
            mode,
            generation: 0,
        }
    }
    #[test]
    fn detection_is_conservative_and_never_probes_or_enables_multiplexer_passthrough() {
        assert!(colors_enabled(None));
        assert!(colors_enabled(Some("")));
        assert!(!colors_enabled(Some("1")));
        assert!(kitty_environment(Some("xterm-kitty"), Some("1"), false));
        for (term, window, multiplexed) in [
            (Some("xterm-256color"), Some("1"), false),
            (Some("xterm-kitty"), None, false),
            (Some("xterm-kitty"), Some("oops"), false),
            (Some("xterm-kitty"), Some("1"), true),
        ] {
            assert!(!kitty_environment(term, window, multiplexed));
        }
    }
    #[test]
    fn embedded_png_and_requested_protocol_sizes_are_bounded() {
        let image = decode(PNG).unwrap();
        assert_eq!((image.width(), image.height()), (512, 512));
        assert!(decode(&vec![0; MAX_ENCODED + 1]).is_err());
        assert!(decode(b"not png").is_err());
        let mut encoded = Cursor::new(Vec::new());
        DynamicImage::new_rgba8(513, 1)
            .write_to(&mut encoded, ImageFormat::Png)
            .unwrap();
        assert!(decode(encoded.get_ref()).is_err());
        for (w, h) in [(0, 0), (29, 14), (28, 15)] {
            let mut k = key(Mode::Cells);
            k.width = w;
            k.height = h;
            assert!(encode(&image, k, None).is_err());
        }
        assert!(PNG.len() < MAX_ENCODED);
        assert!(matches!(
            encode(&image, key(Mode::Kitty), None).unwrap(),
            Protocol::Halfblocks(_)
        ));
        assert!(matches!(
            encode(&image, key(Mode::Kitty), Some((128, 128))).unwrap(),
            Protocol::Halfblocks(_)
        ));
    }
    #[test]
    fn occupied_worker_coalesces_resize_hides_stale_output_and_retains_deadline_slot() {
        let (sender, requests) = sync_channel(1);
        let (results, receiver) = sync_channel(1);
        let mut state = State::new(false);
        state.worker = Some(Worker {
            sender,
            receiver,
            pending: None,
        });
        let first = key(Mode::Cells);
        state.desired = Some(first);
        state.poll();
        assert_eq!(requests.recv().unwrap(), first);
        let mut latest = first;
        for width in 4..=28 {
            latest.width = width;
            state.desired = Some(latest);
            state.poll();
        }
        assert!(
            requests.try_recv().is_err(),
            "no new request while occupied"
        );
        results
            .send((first, encode(&decode(PNG).unwrap(), first, None)))
            .unwrap();
        state.poll();
        assert!(state.ready.is_none());
        assert_eq!(requests.recv().unwrap(), latest);
        state.worker.as_mut().unwrap().pending = Some((latest, Instant::now() - DEADLINE));
        state.poll();
        assert!(state.worker.as_ref().unwrap().pending.is_some());
        state.desired = None;
        results
            .send((latest, encode(&decode(PNG).unwrap(), latest, None)))
            .unwrap();
        state.poll();
        assert!(state.ready.is_none());
        assert!(requests.try_recv().is_err());
    }
    #[test]
    fn kitty_reuses_one_id_hides_without_reupload_and_replaces_payload_before_resize() {
        let source = decode(PNG).unwrap();
        let first = key(Mode::Kitty);
        let mut state = State::new(true);
        state.ready = Some((first, encode(&source, first, Some((8, 16))).unwrap()));
        let mut terminal = Terminal::new(TestBackend::new(40, 20)).unwrap();
        let render = |terminal: &mut Terminal<TestBackend>, state: &mut State, key: Key| {
            terminal
                .draw(|f| {
                    state.begin_frame();
                    state.render(
                        f,
                        Rect::new(2, 2, key.width, key.height),
                        Color::Rgb(20, 24, 33),
                    );
                })
                .unwrap();
            terminal
                .backend()
                .buffer()
                .content
                .iter()
                .map(|c| c.symbol())
                .collect::<String>()
        };
        let initial = render(&mut terminal, &mut state, first);
        assert!(initial.contains("\x1b_G"));
        assert!(
            initial.contains("s=160,v=160"),
            "physical canvas must fit the selected cells"
        );
        assert!(initial.contains(&format!("i={IMAGE_ID}")));
        let repeat = render(&mut terminal, &mut state, first);
        assert!(!repeat.contains("\x1b_G"));
        terminal.draw(|_| state.begin_frame()).unwrap();
        assert!(
            !terminal
                .backend()
                .buffer()
                .content
                .iter()
                .any(|c| c.symbol().contains('\u{10eeee}'))
        );
        let shown = render(&mut terminal, &mut state, first);
        assert!(!shown.contains("\x1b_G"));
        let mut next = first;
        next.width = 24;
        next.height = 12;
        state.ready = Some((next, encode(&source, next, Some((8, 16))).unwrap()));
        let resized = render(&mut terminal, &mut state, next);
        assert!(resized.contains(DELETE));
        assert!(resized.find(DELETE).unwrap() < resized.rfind("\x1b_G").unwrap());
        assert_eq!(state.uploaded, Some(next));
    }
}
