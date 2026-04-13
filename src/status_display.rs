use indicatif::{MultiProgress, ProgressBar, ProgressStyle};

pub struct StatusDisplay {
    progress: ProgressBar,
    stats_engines: ProgressBar,
    stats_result: ProgressBar,
    stats_rate: ProgressBar,
    stats_decide: ProgressBar,
    stats_class: ProgressBar,
    stats_sides: ProgressBar,
}

impl StatusDisplay {
    pub fn new(total_games: usize, prefix: &str) -> Result<Self, String> {
        let mp = MultiProgress::new();
        let progress = mp.add(ProgressBar::new(total_games as u64));
        progress.set_style(
            ProgressStyle::with_template(
                "{prefix} {wide_bar:.green/black} {pos}/{len} ({percent}%) eta {eta_precise}",
            )
            .map_err(|e| e.to_string())?
            .progress_chars("█▉░"),
        );
        progress.set_prefix(prefix.to_string());

        let stats_engines = mp.add(ProgressBar::new_spinner());
        stats_engines.set_style(ProgressStyle::with_template("{msg}").map_err(|e| e.to_string())?);
        let stats_result = mp.add(ProgressBar::new_spinner());
        stats_result.set_style(ProgressStyle::with_template("{msg}").map_err(|e| e.to_string())?);
        let stats_rate = mp.add(ProgressBar::new_spinner());
        stats_rate.set_style(ProgressStyle::with_template("{msg}").map_err(|e| e.to_string())?);
        let stats_decide = mp.add(ProgressBar::new_spinner());
        stats_decide.set_style(ProgressStyle::with_template("{msg}").map_err(|e| e.to_string())?);
        let stats_class = mp.add(ProgressBar::new_spinner());
        stats_class.set_style(ProgressStyle::with_template("{msg}").map_err(|e| e.to_string())?);
        let stats_sides = mp.add(ProgressBar::new_spinner());
        stats_sides.set_style(ProgressStyle::with_template("{msg}").map_err(|e| e.to_string())?);

        Ok(Self {
            progress,
            stats_engines,
            stats_result,
            stats_rate,
            stats_decide,
            stats_class,
            stats_sides,
        })
    }

    pub fn update(&self, done_games: usize, lines: &StatusLines) {
        self.progress.set_position(done_games as u64);
        self.stats_engines.set_message(lines.line_engines.clone());
        self.stats_result.set_message(lines.line_result.clone());
        self.stats_rate.set_message(lines.line_rate.clone());
        self.stats_decide.set_message(lines.line_decide.clone());
        self.stats_class.set_message(lines.line_class.clone());
        self.stats_sides.set_message(lines.line_sides.clone());
    }

    pub fn finish(&self) {
        self.progress.finish_and_clear();
        self.stats_engines.finish_and_clear();
        self.stats_result.finish_and_clear();
        self.stats_rate.finish_and_clear();
        self.stats_decide.finish_and_clear();
        self.stats_class.finish_and_clear();
        self.stats_sides.finish_and_clear();
    }
}

#[derive(Clone)]
pub struct StatusLines {
    pub line_engines: String,
    pub line_result: String,
    pub line_rate: String,
    pub line_decide: String,
    pub line_class: String,
    pub line_sides: String,
}
