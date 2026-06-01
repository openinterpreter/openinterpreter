//! A tiny, self-contained animated "shimmer" status line drawn directly to
//! stderr while the local daemon is cold-starting.
//!
//! The real TUI shimmer (see `codex-tui`'s `shimmer.rs`) is driven by the
//! ratatui frame loop, which is not running yet at this point in startup: the
//! daemon has to be reachable *before* the TUI takes over the screen. So this
//! module reimplements the same moving cosine highlight band over a short
//! string, animated on a background thread, and clears itself once the daemon
//! is ready.

use std::io::IsTerminal;
use std::io::Write;
use std::sync::Arc;
use std::sync::atomic::AtomicBool;
use std::sync::atomic::Ordering;
use std::thread;
use std::thread::JoinHandle;
use std::time::Duration;
use std::time::Instant;

/// The animated label, including the leading bullet icon that the TUI's
/// "Interpreting" indicator also shimmers.
const LABEL: &str = "\u{2022} Starting up...";

/// Frame interval — matches the TUI status indicator's 32ms cadence.
const FRAME: Duration = Duration::from_millis(32);

/// Don't show the startup line until the daemon has been cold-starting for at
/// least this long. Fast startups (often well under a second) finish before
/// this elapses and draw nothing at all, avoiding a distracting flash.
const SHOW_AFTER: Duration = Duration::from_secs(1);

/// A running shimmer animation. Call [`StartupStatus::finish`] once the daemon
/// is ready to stop the animation and clear the line.
pub(crate) struct StartupStatus {
    stop: Arc<AtomicBool>,
    /// Set by the render thread once it actually draws the status line (i.e.
    /// startup outlasted [`SHOW_AFTER`]). `finish` only clears the screen when
    /// this is set, so a fast startup that drew nothing leaves no trace.
    drawn: Arc<AtomicBool>,
    handle: Option<JoinHandle<()>>,
}

impl StartupStatus {
    /// Begin animating the startup line. Returns `None` (and draws nothing) when
    /// stderr is not an interactive terminal — e.g. piped or redirected output.
    pub(crate) fn start() -> Option<Self> {
        if !std::io::stderr().is_terminal() {
            return None;
        }

        let stop = Arc::new(AtomicBool::new(false));
        let drawn = Arc::new(AtomicBool::new(false));
        let stop_for_thread = Arc::clone(&stop);
        let drawn_for_thread = Arc::clone(&drawn);
        let handle = thread::spawn(move || animate(&stop_for_thread, &drawn_for_thread));
        Some(Self {
            stop,
            drawn,
            handle: Some(handle),
        })
    }

    /// Stop the animation, join the render thread, and wipe every trace of the
    /// status line — including the blank line we added above it — so the TUI
    /// (or any subsequent output) starts on a clean screen. If the line was
    /// never drawn (startup finished before [`SHOW_AFTER`]), this leaves the
    /// terminal untouched.
    pub(crate) fn finish(mut self) {
        self.stop.store(true, Ordering::Relaxed);
        if let Some(handle) = self.handle.take() {
            let _ = handle.join();
        }

        // Nothing was drawn (fast startup) — don't touch the terminal.
        if !self.drawn.load(Ordering::Relaxed) {
            return;
        }

        let mut stderr = std::io::stderr().lock();
        // Clear the animated line, move back up over the leading newline, and
        // clear that line too, leaving the cursor exactly where we started.
        let _ = write!(stderr, "\r\x1b[2K\x1b[1A\r\x1b[2K");
        let _ = stderr.flush();
    }
}

fn animate(stop: &AtomicBool, drawn: &AtomicBool) {
    let truecolor = supports_truecolor();
    let start = Instant::now();

    // Hold off drawing anything until startup has lasted SHOW_AFTER. Poll the
    // stop flag at the frame cadence so a fast startup exits promptly without
    // ever touching the terminal.
    while start.elapsed() < SHOW_AFTER {
        if stop.load(Ordering::Relaxed) {
            return;
        }
        thread::sleep(FRAME);
    }
    if stop.load(Ordering::Relaxed) {
        return;
    }

    // Past the threshold: commit to drawing. Mark `drawn` before any output so
    // `finish` knows it must clear the line afterward.
    drawn.store(true, Ordering::Relaxed);
    let mut stderr = std::io::stderr().lock();
    // One newline of separation above the status line, drawn once.
    let _ = write!(stderr, "\n");

    while !stop.load(Ordering::Relaxed) {
        let frame = render_frame(start.elapsed(), truecolor);
        // Carriage return + clear-to-end-of-line, then the frame.
        let _ = write!(stderr, "\r\x1b[2K{frame}");
        let _ = stderr.flush();
        thread::sleep(FRAME);
    }
}

/// Render a single frame of the shimmer as an ANSI-styled string.
///
/// Mirrors the math in `codex-tui::shimmer::shimmer_spans`: a cosine-shaped
/// highlight band of `band_half_width` sweeps across the characters once every
/// `sweep_seconds`, brightening the base color toward a highlight color.
fn render_frame(elapsed: Duration, truecolor: bool) -> String {
    let chars: Vec<char> = LABEL.chars().collect();
    let padding = 10usize;
    let period = chars.len() + padding * 2;
    let sweep_seconds = 2.0f32;
    let pos = ((elapsed.as_secs_f32() % sweep_seconds) / sweep_seconds * period as f32) as isize;
    let band_half_width = 5.0f32;

    // Dim gray base that brightens toward near-white at the band's peak.
    let base = (130u8, 130u8, 130u8);
    let highlight = (235u8, 235u8, 235u8);

    let mut out = String::with_capacity(chars.len() * 20);
    for (i, ch) in chars.iter().enumerate() {
        let dist = ((i as isize + padding as isize) - pos).abs() as f32;
        let t = if dist <= band_half_width {
            0.5 * (1.0 + (std::f32::consts::PI * (dist / band_half_width)).cos())
        } else {
            0.0
        };

        if truecolor {
            let (r, g, b) = lerp(base, highlight, t.clamp(0.0, 1.0) * 0.9);
            out.push_str(&format!("\x1b[1;38;2;{r};{g};{b}m{ch}"));
        } else if t > 0.6 {
            out.push_str(&format!("\x1b[1m{ch}\x1b[22m"));
        } else if t < 0.2 {
            out.push_str(&format!("\x1b[2m{ch}\x1b[22m"));
        } else {
            out.push(*ch);
        }
    }
    out.push_str("\x1b[0m");
    out
}

fn lerp(from: (u8, u8, u8), to: (u8, u8, u8), t: f32) -> (u8, u8, u8) {
    let mix = |a: u8, b: u8| (a as f32 + (b as f32 - a as f32) * t).round() as u8;
    (mix(from.0, to.0), mix(from.1, to.1), mix(from.2, to.2))
}

fn supports_truecolor() -> bool {
    std::env::var("COLORTERM")
        .map(|v| v.contains("truecolor") || v.contains("24bit"))
        .unwrap_or(false)
}
