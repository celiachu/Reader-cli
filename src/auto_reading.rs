use std::time::{Duration, Instant};

/// 自动阅读的工作模式
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum AutoReadingMode {
    /// 定时自动翻页（默认黄金模式：页面静止，根据当前页字数动态停留后翻页）
    #[default]
    TimedPage,
    /// 连续行推移滚屏（终端实验模式：按行平缓推进文本）
    LineScroll,
}

impl AutoReadingMode {
    pub fn label(&self) -> &'static str {
        match self {
            Self::TimedPage => "Page",
            Self::LineScroll => "Scroll",
        }
    }
}

/// 页面停留时间下限保底（避免空页或极短页面一闪而过）
pub const MIN_PAGE_DURATION: Duration = Duration::from_millis(3500);
/// 页面停留时间上限封顶（单页最长 5 分钟）
pub const MAX_PAGE_DURATION: Duration = Duration::from_secs(300);
/// 新章节开头缓冲奖励时间（章节转换时读者认知负荷增加，额外停留缓冲）
pub const CHAPTER_START_BONUS: Duration = Duration::from_millis(1800);

/// 单行停留时间下限保底（滚动模式下每行最少停留时间）
pub const MIN_LINE_DURATION: Duration = Duration::from_millis(1000);
/// 单行停留时间上限（滚动模式下单行最多停留时间）
pub const MAX_LINE_DURATION: Duration = Duration::from_secs(15);

/// 判断字符是否为中日韩（CJK）统一表意字符或假名/谚文字符
pub fn is_cjk_character(ch: char) -> bool {
    matches!(ch as u32,
        0x4E00..=0x9FFF       // CJK Unified Ideographs
        | 0x3400..=0x4DBF     // CJK Unified Ideographs Extension A
        | 0x20000..=0x2A6DF   // CJK Extension B
        | 0x2A700..=0x2B73F   // CJK Extension C
        | 0x2B740..=0x2B81F   // CJK Extension D
        | 0x2B820..=0x2CEAF   // CJK Extension E
        | 0xF900..=0xFAFF     // CJK Compatibility Ideographs
        | 0x3040..=0x309F     // Hiragana
        | 0x30A0..=0x30FF     // Katakana
        | 0xAC00..=0xD7AF     // Hangul Syllables
    )
}

/// 统计单行文本中的有效阅读单元数量（CPM 换算基准）：
/// - CJK 汉字/假名每个计 1 个阅读单元
/// - 西文单词（连续字母数字）按词长折算：约每 5 个字符计 1 个阅读单元（至少 1 个）
/// - 标点符号计 1 个阅读单元
/// - 空格、制表符与控制字符忽略不计
pub fn count_effective_reading_chars_in_line(line: &str) -> usize {
    let mut count = 0;
    let mut word_len = 0;

    for ch in line.chars() {
        if is_cjk_character(ch) {
            if word_len > 0 {
                count += (word_len / 5).max(1);
                word_len = 0;
            }
            count += 1;
        } else if ch.is_ascii_alphanumeric() || (ch.is_alphabetic() && !ch.is_ascii()) {
            word_len += 1;
        } else {
            if word_len > 0 {
                count += (word_len / 5).max(1);
                word_len = 0;
            }
            if !ch.is_whitespace() && !ch.is_control() {
                count += 1;
            }
        }
    }
    if word_len > 0 {
        count += (word_len / 5).max(1);
    }
    count
}

/// 计算页面切片中的有效阅读字数，剔除上一页的重叠行（overlap_lines）
pub fn count_effective_reading_chars(lines: &[String], overlap_lines: usize) -> usize {
    lines
        .iter()
        .skip(overlap_lines)
        .map(|line| count_effective_reading_chars_in_line(line))
        .sum()
}

/// 根据目标阅读速度（CPM: 字/分钟）计算当前页面建议阅读停留时间：
/// 公式：T = (C / R) * 60 秒
/// - C: 本页实际有效字数
/// - R: 目标阅读速度 (CPM)
/// - 加成项: 如果为章节开头则追加缓冲时长
/// - 下限保底: 不低于 MIN_PAGE_DURATION (3.5s)
/// - 上限保护: 不高于 MAX_PAGE_DURATION (300s)
pub fn calculate_page_duration(
    cpm: u32,
    effective_chars: usize,
    is_chapter_start: bool,
) -> Duration {
    let speed = cpm.clamp(60, 2000) as f64;
    let base_seconds = (effective_chars as f64 / speed) * 60.0;
    let mut duration = Duration::from_secs_f64(base_seconds);
    if is_chapter_start {
        duration += CHAPTER_START_BONUS;
    }
    duration.clamp(MIN_PAGE_DURATION, MAX_PAGE_DURATION)
}

/// 根据目标阅读速度（CPM）计算单行停留时间（供行滚动模式使用）
pub fn calculate_line_duration(cpm: u32, line: &str) -> Duration {
    let effective = count_effective_reading_chars_in_line(line);
    if effective == 0 {
        return Duration::from_millis(800);
    }
    let speed = cpm.clamp(60, 2000) as f64;
    let seconds = (effective as f64 / speed) * 60.0;
    Duration::from_secs_f64(seconds).clamp(MIN_LINE_DURATION, MAX_LINE_DURATION)
}

/// 样本数据：记录读者手动阅读某一页的字数与真实用时
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct CalibrationSample {
    pub char_count: usize,
    pub elapsed: Duration,
    pub cpm: u32,
}

/// 阅读速度自然校准器：通过读者正常手动翻阅 2-3 页的数据，自动测算真实阅读节奏
#[derive(Debug, Clone, Default)]
pub struct SpeedCalibration {
    samples: Vec<CalibrationSample>,
    current_page_enter: Option<(Instant, usize)>,
}

impl SpeedCalibration {
    /// 当读者进入新页面时记录进入时间与有效字数
    pub fn on_page_entered(&mut self, char_count: usize, now: Instant) {
        self.current_page_enter = Some((now, char_count));
    }

    /// 当读者手动翻页离开时，结算上一页阅读样本（过滤过短的误触与过长的挂机）
    pub fn on_manual_page_turn(&mut self, now: Instant) -> Option<CalibrationSample> {
        let (entered_at, char_count) = self.current_page_enter.take()?;
        let elapsed = now.saturating_duration_since(entered_at);
        // 过滤噪声：至少阅读 3 秒，最多 180 秒，且该页有效字数 >= 40
        if elapsed < Duration::from_secs(3) || elapsed > Duration::from_secs(180) || char_count < 40
        {
            return None;
        }
        let secs = elapsed.as_secs_f64();
        let cpm = ((char_count as f64 / secs) * 60.0).round() as u32;
        if !(80..=1200).contains(&cpm) {
            return None;
        }
        let sample = CalibrationSample {
            char_count,
            elapsed,
            cpm,
        };
        self.samples.push(sample);
        if self.samples.len() > 10 {
            self.samples.remove(0);
        }
        Some(sample)
    }

    /// 当前已采集的有效样本数量
    pub fn sample_count(&self) -> usize {
        self.samples.len()
    }

    /// 清空样本数据
    pub fn clear(&mut self) {
        self.samples.clear();
        self.current_page_enter = None;
    }

    /// 计算校准后的阅读速度：
    /// 取样本中位数，并乘以 0.90（略微保守的 90% 节奏，避免读者有被追赶的压迫感）
    pub fn compute_calibrated_cpm(&self) -> Option<u32> {
        if self.samples.is_empty() {
            return None;
        }
        let mut cpms: Vec<u32> = self.samples.iter().map(|s| s.cpm).collect();
        cpms.sort_unstable();
        let mid = cpms.len() / 2;
        let median = if cpms.len().is_multiple_of(2) {
            (cpms[mid - 1] + cpms[mid]) / 2
        } else {
            cpms[mid]
        };
        let suggested = ((median as f64) * 0.90).round() as u32;
        Some(suggested.clamp(100, 1000))
    }
}

/// 自动阅读运行时状态机
#[derive(Debug, Clone)]
pub struct AutoReadingState {
    /// 自动阅读是否已启用
    pub active: bool,
    /// 是否处于暂停状态
    pub paused: bool,
    /// 自动阅读模式（翻页 vs 滚屏）
    pub mode: AutoReadingMode,
    /// 目标阅读速度（CPM: 字/分钟）
    pub cpm: u32,

    /// 当前页计时开始时间戳
    pub timer_started: Instant,
    /// 当前页目标停留总时长
    pub target_duration: Duration,
    /// 暂停前已流逝的累计时长（恢复时无缝衔接）
    pub elapsed_before_pause: Duration,

    /// 阅读速度自然校准器
    pub calibration: SpeedCalibration,

    /// 行推移滚屏状态（Phase 3）
    pub line_timer_started: Instant,
    pub line_target_duration: Duration,
    pub line_elapsed_before_pause: Duration,
}

impl AutoReadingState {
    pub fn new(cpm: u32, mode: AutoReadingMode) -> Self {
        let now = Instant::now();
        Self {
            active: false,
            paused: false,
            mode,
            cpm: cpm.clamp(60, 2000),
            timer_started: now,
            target_duration: MIN_PAGE_DURATION,
            elapsed_before_pause: Duration::ZERO,
            calibration: SpeedCalibration::default(),
            line_timer_started: now,
            line_target_duration: MIN_LINE_DURATION,
            line_elapsed_before_pause: Duration::ZERO,
        }
    }

    /// 启动自动阅读
    pub fn start(
        &mut self,
        page_chars: usize,
        is_chapter_start: bool,
        first_line: Option<&str>,
        now: Instant,
    ) {
        self.active = true;
        self.paused = false;
        self.reset_for_page(page_chars, is_chapter_start, now);
        if let Some(line) = first_line {
            self.reset_for_line(line, now);
        }
    }

    /// 停止自动阅读
    pub fn stop(&mut self) {
        self.active = false;
        self.paused = false;
        self.elapsed_before_pause = Duration::ZERO;
        self.line_elapsed_before_pause = Duration::ZERO;
    }

    /// 切换暂停/恢复状态（Space 键）
    pub fn toggle_pause(&mut self, now: Instant) -> bool {
        if !self.active {
            return false;
        }
        if self.paused {
            self.resume(now);
            false
        } else {
            self.pause(now);
            true
        }
    }

    /// 暂停计时
    pub fn pause(&mut self, now: Instant) {
        if self.active && !self.paused {
            self.elapsed_before_pause += now.saturating_duration_since(self.timer_started);
            self.line_elapsed_before_pause +=
                now.saturating_duration_since(self.line_timer_started);
            self.paused = true;
        }
    }

    /// 恢复计时
    pub fn resume(&mut self, now: Instant) {
        if self.active && self.paused {
            self.timer_started = now;
            self.line_timer_started = now;
            self.paused = false;
        }
    }

    /// 当前页已阅读时长
    pub fn elapsed(&self, now: Instant) -> Duration {
        if !self.active {
            return Duration::ZERO;
        }
        if self.paused {
            self.elapsed_before_pause
        } else {
            self.elapsed_before_pause + now.saturating_duration_since(self.timer_started)
        }
    }

    /// 行推移模式下当前行已停留时长
    pub fn line_elapsed(&self, now: Instant) -> Duration {
        if !self.active {
            return Duration::ZERO;
        }
        if self.paused {
            self.line_elapsed_before_pause
        } else {
            self.line_elapsed_before_pause + now.saturating_duration_since(self.line_timer_started)
        }
    }

    /// 当前页剩余停留时间
    pub fn remaining_time(&self, now: Instant) -> Duration {
        let elapsed = self.elapsed(now);
        self.target_duration.saturating_sub(elapsed)
    }

    /// 当前页阅读时间完成比例 (0.0 ..= 1.0)
    pub fn progress_ratio(&self, now: Instant) -> f32 {
        if self.target_duration.is_zero() {
            return 1.0;
        }
        let elapsed = self.elapsed(now).as_secs_f32();
        let target = self.target_duration.as_secs_f32();
        (elapsed / target).clamp(0.0, 1.0)
    }

    /// 检查当前页是否已到达翻页时间
    pub fn is_page_due(&self, now: Instant) -> bool {
        self.active && !self.paused && self.elapsed(now) >= self.target_duration
    }

    /// 行推移模式下：检查当前行是否已到达推移行时间
    pub fn is_line_due(&self, now: Instant) -> bool {
        self.active
            && !self.paused
            && self.mode == AutoReadingMode::LineScroll
            && self.line_elapsed(now) >= self.line_target_duration
    }

    /// 页面重置（翻至新页或手动调整时，重置页面计时器）
    pub fn reset_for_page(&mut self, page_chars: usize, is_chapter_start: bool, now: Instant) {
        self.target_duration = calculate_page_duration(self.cpm, page_chars, is_chapter_start);
        self.timer_started = now;
        self.elapsed_before_pause = Duration::ZERO;
    }

    /// 单行重置（行推移滚动至新行时重置单行计时器）
    pub fn reset_for_line(&mut self, line: &str, now: Instant) {
        self.line_target_duration = calculate_line_duration(self.cpm, line);
        self.line_timer_started = now;
        self.line_elapsed_before_pause = Duration::ZERO;
    }

    /// 实时调整阅读速度（CPM: +/- 步长）
    /// 并按剩余阅读进度等比例缩放当前页的剩余时长
    pub fn adjust_cpm(
        &mut self,
        delta: i32,
        page_chars: usize,
        is_chapter_start: bool,
        now: Instant,
    ) -> u32 {
        let new_cpm = (self.cpm as i32 + delta).clamp(60, 2000) as u32;
        self.set_cpm(new_cpm, page_chars, is_chapter_start, now);
        self.cpm
    }

    /// 直接设定阅读速度
    pub fn set_cpm(&mut self, cpm: u32, page_chars: usize, is_chapter_start: bool, now: Instant) {
        let old_ratio = self.progress_ratio(now);
        self.cpm = cpm.clamp(60, 2000);
        self.target_duration = calculate_page_duration(self.cpm, page_chars, is_chapter_start);
        let new_elapsed = self.target_duration.mul_f32(old_ratio);
        self.elapsed_before_pause = new_elapsed;
        self.timer_started = now;
    }

    /// 应用校准测算出的 CPM
    pub fn apply_calibrated_cpm(
        &mut self,
        page_chars: usize,
        is_chapter_start: bool,
        now: Instant,
    ) -> Option<u32> {
        let calibrated = self.calibration.compute_calibrated_cpm()?;
        self.set_cpm(calibrated, page_chars, is_chapter_start, now);
        Some(calibrated)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn counts_cjk_and_punctuation_accurately() {
        let line = "《地下室手记》是一本好书。";
        // 《 (1) + 地 (1) + 下 (1) + 室 (1) + 手 (1) + 记 (1) + 》 (1) + 是 (1) + 一 (1) + 本 (1) + 好 (1) + 书 (1) + 。 (1) = 13
        assert_eq!(count_effective_reading_chars_in_line(line), 13);
    }

    #[test]
    fn groups_latin_words_by_cognitive_units() {
        let line = "Hello world!";
        // Hello (5 chars => 1 unit), world (5 chars => 1 unit), ! (1 unit) => 3
        assert_eq!(count_effective_reading_chars_in_line(line), 3);

        let mixed = "阅读 Rust 语言与 Tokio 异步框架。";
        // 阅(1) 读(1) Rust(1) 语(1) 言(1) 与(1) Tokio(1) 异(1) 步(1) 框(1) 架(1) 。(1) = 12
        assert_eq!(count_effective_reading_chars_in_line(mixed), 12);
    }

    #[test]
    fn ignores_whitespace_and_skips_overlap_lines() {
        let lines = vec![
            "    第一行上一页残留   ".to_string(), // overlap line, should be skipped
            "第二行真实正文内容。".to_string(),    // 10 units
            "".to_string(),
            "   第三行末尾。".to_string(), // 6 units (第、三、行、末、尾、。)
        ];
        let count = count_effective_reading_chars(&lines, 1);
        assert_eq!(count, 16);
    }

    #[test]
    fn page_duration_calculation_matches_user_research_formula() {
        // T = (C / R) * 60
        // 300 CPM, 450 chars => 90 seconds (1m 30s)
        let d = calculate_page_duration(300, 450, false);
        assert_eq!(d, Duration::from_secs(90));

        // 300 CPM, 200 chars => 40 seconds
        let d = calculate_page_duration(300, 200, false);
        assert_eq!(d, Duration::from_secs(40));

        // 300 CPM, 350 chars => 70 seconds
        let d = calculate_page_duration(300, 350, false);
        assert_eq!(d, Duration::from_secs(70));

        // 300 CPM, 500 chars => 100 seconds
        let d = calculate_page_duration(300, 500, false);
        assert_eq!(d, Duration::from_secs(100));

        // 300 CPM, 700 chars => 140 seconds
        let d = calculate_page_duration(300, 700, false);
        assert_eq!(d, Duration::from_secs(140));
    }

    #[test]
    fn applies_minimum_floor_and_chapter_bonus() {
        // Very short page: 5 chars => 1.0s, but MIN_PAGE_DURATION = 3.5s
        let d = calculate_page_duration(300, 5, false);
        assert_eq!(d, MIN_PAGE_DURATION);

        // Chapter start adds bonus
        let d_normal = calculate_page_duration(300, 200, false);
        let d_chapter = calculate_page_duration(300, 200, true);
        assert_eq!(d_chapter, d_normal + CHAPTER_START_BONUS);
    }

    #[test]
    fn pause_and_resume_preserves_elapsed_time() {
        let mut state = AutoReadingState::new(300, AutoReadingMode::TimedPage);
        let now = Instant::now();
        state.start(300, false, None, now);
        assert_eq!(state.target_duration, Duration::from_secs(60));

        // Advance 10s then pause
        let at_10s = now + Duration::from_secs(10);
        state.pause(at_10s);
        assert!(state.paused);
        assert_eq!(state.elapsed(at_10s), Duration::from_secs(10));
        assert_eq!(state.remaining_time(at_10s), Duration::from_secs(50));

        // Pause for 5 minutes (user walked away)
        let at_310s = at_10s + Duration::from_secs(300);
        assert_eq!(state.elapsed(at_310s), Duration::from_secs(10));

        // Resume at 310s
        state.resume(at_310s);
        assert!(!state.paused);

        // 5s after resume
        let at_315s = at_310s + Duration::from_secs(5);
        assert_eq!(state.elapsed(at_315s), Duration::from_secs(15));
        assert_eq!(state.remaining_time(at_315s), Duration::from_secs(45));
    }

    #[test]
    fn speed_calibration_computes_90_percent_of_median() {
        let mut cal = SpeedCalibration::default();
        let base = Instant::now();

        // Page 1: 300 chars in 60s => 300 CPM
        cal.on_page_entered(300, base);
        let s1 = cal.on_manual_page_turn(base + Duration::from_secs(60));
        assert!(s1.is_some());
        assert_eq!(s1.unwrap().cpm, 300);

        // Page 2: 400 chars in 60s => 400 CPM
        let p2_start = base + Duration::from_secs(60);
        cal.on_page_entered(400, p2_start);
        let s2 = cal.on_manual_page_turn(p2_start + Duration::from_secs(60));
        assert!(s2.is_some());
        assert_eq!(s2.unwrap().cpm, 400);

        // Page 3: 350 chars in 60s => 350 CPM
        let p3_start = p2_start + Duration::from_secs(60);
        cal.on_page_entered(350, p3_start);
        let s3 = cal.on_manual_page_turn(p3_start + Duration::from_secs(60));
        assert!(s3.is_some());
        assert_eq!(s3.unwrap().cpm, 350);

        // Median of [300, 350, 400] is 350.
        // 90% of 350 = 315 CPM!
        let calibrated = cal.compute_calibrated_cpm().unwrap();
        assert_eq!(calibrated, 315);
    }

    #[test]
    fn speed_calibration_filters_skims_and_inactivity() {
        let mut cal = SpeedCalibration::default();
        let base = Instant::now();

        // Skim: 1 second (< 3s)
        cal.on_page_entered(300, base);
        assert!(cal
            .on_manual_page_turn(base + Duration::from_secs(1))
            .is_none());

        // Inactivity: 5 minutes (> 180s)
        cal.on_page_entered(300, base);
        assert!(cal
            .on_manual_page_turn(base + Duration::from_secs(300))
            .is_none());

        // Too few chars: 10 chars
        cal.on_page_entered(10, base);
        assert!(cal
            .on_manual_page_turn(base + Duration::from_secs(10))
            .is_none());
    }

    #[test]
    fn adjust_cpm_preserves_progress_ratio() {
        let mut state = AutoReadingState::new(300, AutoReadingMode::TimedPage);
        let now = Instant::now();
        state.start(300, false, None, now);
        assert_eq!(state.target_duration, Duration::from_secs(60));

        // Read half of the page (30s out of 60s => 50%)
        let half_time = now + Duration::from_secs(30);
        assert!((state.progress_ratio(half_time) - 0.5).abs() < 0.01);

        // Adjust speed up to 600 CPM (new target = 30s)
        state.adjust_cpm(300, 300, false, half_time);
        assert_eq!(state.cpm, 600);
        assert_eq!(state.target_duration, Duration::from_secs(30));
        // Progress ratio should remain ~50%
        assert!((state.progress_ratio(half_time) - 0.5).abs() < 0.01);
        // Remaining time should now be 15s instead of 30s
        assert_eq!(state.remaining_time(half_time).as_secs(), 15);
    }
}
