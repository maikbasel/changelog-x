use std::cell::Cell;
use std::time::Instant;

use console::style;
use indicatif::{MultiProgress, ProgressBar, ProgressStyle};

// indicatif template placeholders look like Rust format args but are not.
#[allow(clippy::literal_string_with_formatting_args)]
const STATIC_TPL: &str = "{prefix} {msg}";
#[allow(clippy::literal_string_with_formatting_args)]
const SPINNER_TPL: &str = "{prefix} {spinner:.green} {msg}";

/// Build a `ProgressStyle` from a known-valid template.
///
/// All templates used here are compile-time literals that never fail.
fn style_from(template: &str) -> ProgressStyle {
    ProgressStyle::with_template(template).unwrap_or_else(|_| ProgressStyle::default_spinner())
}

/// A multi-step progress pipeline displayed on stderr.
///
/// Each step is shown as a numbered line with a spinner (active),
/// checkmark (done), or cross (failed).
pub struct Pipeline {
    #[allow(dead_code)]
    mp: MultiProgress,
    bars: Vec<ProgressBar>,
    labels: Vec<String>,
    total: usize,
    current: Cell<Option<usize>>,
    start: Instant,
}

impl Pipeline {
    /// Create a new pipeline with the given step labels.
    ///
    /// All steps start in a pending (dim) state.
    #[must_use]
    pub fn new(steps: &[&str]) -> Self {
        let total = steps.len();
        let mp = MultiProgress::new();
        let mut bars = Vec::with_capacity(total);
        let labels: Vec<String> = steps.iter().map(|s| (*s).to_owned()).collect();

        for (i, label) in labels.iter().enumerate() {
            let pb = mp.add(ProgressBar::new_spinner());
            let prefix = format!("  [{}/{}]", i + 1, total);
            pb.set_style(style_from(STATIC_TPL));
            pb.set_prefix(prefix);
            pb.set_message(format!("{}", style(label).dim()));
            bars.push(pb);
        }

        Self {
            mp,
            bars,
            labels,
            total,
            current: Cell::new(None),
            start: Instant::now(),
        }
    }

    /// Finish the current step with a checkmark and activate the next step with a spinner.
    ///
    /// On the first call, activates step 0. On subsequent calls, finishes the
    /// current step and activates the next one.
    pub fn advance(&self) {
        let next = self.current.get().map_or(0, |i| {
            self.mark_done(i);
            i + 1
        });

        if next < self.total {
            self.activate(next);
            self.current.set(Some(next));
        }
    }

    /// Mark the last active step as done and print a timing summary.
    pub fn finish_all(&self) {
        if let Some(i) = self.current.get() {
            self.mark_done(i);
        }

        let elapsed = self.start.elapsed();
        let secs = elapsed.as_secs();
        let centis = elapsed.subsec_millis() / 10;
        let summary = self.mp.add(ProgressBar::new_spinner());
        summary.set_style(style_from(STATIC_TPL));
        summary.set_prefix(format!("{}", style("\u{26a1}").yellow()));
        summary.set_message(format!(
            "{}",
            style(format!("Done in {secs}.{centis:02}s")).bold()
        ));
        summary.finish();
    }

    /// Mark the current step as failed with a red cross and message.
    pub fn fail(&self, msg: &str) {
        if let Some(i) = self.current.get()
            && let Some(pb) = self.bars.get(i)
        {
            let prefix = format!(
                "  [{}/{}] {}",
                i + 1,
                self.total,
                style("\u{2717}").red().bold()
            );
            pb.set_style(style_from(STATIC_TPL));
            pb.set_prefix(prefix);
            pb.set_message(format!("{}", style(msg).red()));
            pb.finish();
        }
    }

    fn activate(&self, idx: usize) {
        if let Some(pb) = self.bars.get(idx) {
            let prefix = format!("  [{}/{}]", idx + 1, self.total);
            pb.set_style(style_from(SPINNER_TPL).tick_strings(&[
                "\u{2800}", "\u{2801}", "\u{2803}", "\u{2807}", "\u{280f}", "\u{281f}", "\u{283f}",
                "\u{287f}", "\u{28ff}", "\u{28fe}", "\u{28fc}", "\u{28f8}", "\u{28f0}", "\u{28e0}",
                "\u{28c0}", "\u{2880}", "\u{2800}",
            ]));
            pb.set_prefix(prefix);
            pb.set_message(format!("{}...", self.labels[idx]));
            pb.enable_steady_tick(std::time::Duration::from_millis(80));
        }
    }

    fn mark_done(&self, idx: usize) {
        if let Some(pb) = self.bars.get(idx) {
            let prefix = format!(
                "  [{}/{}] {}",
                idx + 1,
                self.total,
                style("\u{2714}").green().bold()
            );
            pb.set_style(style_from(STATIC_TPL));
            pb.set_prefix(prefix);
            pb.set_message(self.labels[idx].clone());
            pb.finish();
        }
    }
}
