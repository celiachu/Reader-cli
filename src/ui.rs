use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use ratatui::layout::{Constraint, Direction, Layout, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::Block;
use unicode_width::UnicodeWidthStr;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ReadingTheme {
    #[default]
    Terminal,
    Sepia,
    Eink,
    Nord,
    Forest,
    Dracula,
    Rose,
    Solarized,
}

impl ReadingTheme {
    pub fn next(self) -> Self {
        match self {
            Self::Terminal => Self::Sepia,
            Self::Sepia => Self::Eink,
            Self::Eink => Self::Nord,
            Self::Nord => Self::Forest,
            Self::Forest => Self::Dracula,
            Self::Dracula => Self::Rose,
            Self::Rose => Self::Solarized,
            Self::Solarized => Self::Terminal,
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Self::Terminal => "Terminal Default (终端原生)",
            Self::Sepia => "Warm Sepia (羊皮纸暖色)",
            Self::Eink => "Paper E-ink (纸质水墨屏)",
            Self::Nord => "Nord Dark (极光夜)",
            Self::Forest => "Forest Mint (青竹静谧绿)",
            Self::Dracula => "Dracula Dark (赛博暗夜)",
            Self::Rose => "Rose Pine (晚霞落樱)",
            Self::Solarized => "Solarized Light (日光宣纸)",
        }
    }
}

#[derive(Debug, Clone, Copy)]
pub struct Palette {
    pub background: Color,
    pub body: Color,
    pub muted: Color,
    pub accent: Color,
    pub discreet_body: Color,
    pub discreet_muted: Color,
    pub discreet_auxiliary: Color,
    pub selected_foreground: Color,
    pub selected_background: Color,
    pub border: Color,
    pub error: Color,
    pub success: Color,
}

impl Palette {
    pub const fn terminal() -> Self {
        Self {
            background: Color::Reset,
            body: Color::Reset,
            muted: Color::DarkGray,
            accent: Color::Cyan,
            discreet_body: Color::Reset,
            discreet_muted: Color::DarkGray,
            discreet_auxiliary: Color::DarkGray,
            selected_foreground: Color::Reset,
            selected_background: Color::Reset,
            border: Color::DarkGray,
            error: Color::Red,
            success: Color::Green,
        }
    }

    pub const fn sepia() -> Self {
        Self {
            background: Color::Rgb(30, 28, 25),
            body: Color::Rgb(232, 220, 190),
            muted: Color::Rgb(140, 130, 115),
            accent: Color::Rgb(215, 155, 90),
            discreet_body: Color::Rgb(232, 220, 190),
            discreet_muted: Color::Rgb(140, 130, 115),
            discreet_auxiliary: Color::Rgb(215, 155, 90),
            selected_foreground: Color::Rgb(30, 28, 25),
            selected_background: Color::Rgb(232, 220, 190),
            border: Color::Rgb(70, 65, 55),
            error: Color::Rgb(230, 95, 80),
            success: Color::Rgb(140, 180, 100),
        }
    }

    pub const fn eink() -> Self {
        Self {
            background: Color::Rgb(24, 24, 24),
            body: Color::Rgb(210, 210, 210),
            muted: Color::Rgb(110, 110, 110),
            accent: Color::Rgb(240, 240, 240),
            discreet_body: Color::Rgb(210, 210, 210),
            discreet_muted: Color::Rgb(110, 110, 110),
            discreet_auxiliary: Color::Rgb(180, 180, 180),
            selected_foreground: Color::Rgb(24, 24, 24),
            selected_background: Color::Rgb(210, 210, 210),
            border: Color::Rgb(60, 60, 60),
            error: Color::Rgb(220, 80, 80),
            success: Color::Rgb(160, 160, 160),
        }
    }

    pub const fn nord() -> Self {
        Self {
            background: Color::Rgb(46, 52, 64),
            body: Color::Rgb(236, 239, 244),
            muted: Color::Rgb(129, 161, 193),
            accent: Color::Rgb(136, 192, 208),
            discreet_body: Color::Rgb(236, 239, 244),
            discreet_muted: Color::Rgb(129, 161, 193),
            discreet_auxiliary: Color::Rgb(136, 192, 208),
            selected_foreground: Color::Rgb(46, 52, 64),
            selected_background: Color::Rgb(236, 239, 244),
            border: Color::Rgb(76, 86, 106),
            error: Color::Rgb(191, 97, 106),
            success: Color::Rgb(163, 190, 140),
        }
    }

    pub const fn forest() -> Self {
        Self {
            background: Color::Rgb(24, 34, 28),
            body: Color::Rgb(216, 232, 220),
            muted: Color::Rgb(120, 155, 130),
            accent: Color::Rgb(130, 205, 155),
            discreet_body: Color::Rgb(216, 232, 220),
            discreet_muted: Color::Rgb(120, 155, 130),
            discreet_auxiliary: Color::Rgb(130, 205, 155),
            selected_foreground: Color::Rgb(24, 34, 28),
            selected_background: Color::Rgb(130, 205, 155),
            border: Color::Rgb(48, 72, 58),
            error: Color::Rgb(230, 105, 95),
            success: Color::Rgb(140, 210, 130),
        }
    }

    pub const fn dracula() -> Self {
        Self {
            background: Color::Rgb(40, 42, 54),
            body: Color::Rgb(248, 248, 242),
            muted: Color::Rgb(98, 114, 164),
            accent: Color::Rgb(189, 147, 249),
            discreet_body: Color::Rgb(248, 248, 242),
            discreet_muted: Color::Rgb(98, 114, 164),
            discreet_auxiliary: Color::Rgb(139, 233, 253),
            selected_foreground: Color::Rgb(40, 42, 54),
            selected_background: Color::Rgb(189, 147, 249),
            border: Color::Rgb(68, 71, 90),
            error: Color::Rgb(255, 85, 85),
            success: Color::Rgb(80, 250, 123),
        }
    }

    pub const fn rose() -> Self {
        Self {
            background: Color::Rgb(31, 29, 46),
            body: Color::Rgb(224, 222, 244),
            muted: Color::Rgb(144, 140, 170),
            accent: Color::Rgb(235, 188, 186),
            discreet_body: Color::Rgb(224, 222, 244),
            discreet_muted: Color::Rgb(144, 140, 170),
            discreet_auxiliary: Color::Rgb(235, 188, 186),
            selected_foreground: Color::Rgb(31, 29, 46),
            selected_background: Color::Rgb(235, 188, 186),
            border: Color::Rgb(68, 65, 90),
            error: Color::Rgb(235, 111, 146),
            success: Color::Rgb(156, 207, 216),
        }
    }

    pub const fn solarized() -> Self {
        Self {
            background: Color::Rgb(253, 246, 227),
            body: Color::Rgb(88, 110, 117),
            muted: Color::Rgb(147, 161, 161),
            accent: Color::Rgb(38, 139, 210),
            discreet_body: Color::Rgb(88, 110, 117),
            discreet_muted: Color::Rgb(147, 161, 161),
            discreet_auxiliary: Color::Rgb(38, 139, 210),
            selected_foreground: Color::Rgb(253, 246, 227),
            selected_background: Color::Rgb(38, 139, 210),
            border: Color::Rgb(220, 215, 195),
            error: Color::Rgb(220, 50, 47),
            success: Color::Rgb(133, 153, 0),
        }
    }

    pub fn for_reading_theme(theme: ReadingTheme) -> Self {
        match theme {
            ReadingTheme::Terminal => Self::terminal(),
            ReadingTheme::Sepia => Self::sepia(),
            ReadingTheme::Eink => Self::eink(),
            ReadingTheme::Nord => Self::nord(),
            ReadingTheme::Forest => Self::forest(),
            ReadingTheme::Dracula => Self::dracula(),
            ReadingTheme::Rose => Self::rose(),
            ReadingTheme::Solarized => Self::solarized(),
        }
    }

    pub fn base(self) -> Style {
        Style::default().fg(self.body).bg(self.background)
    }

    pub const fn discreet_mode(self) -> Self {
        Self {
            background: self.background,
            body: self.discreet_body,
            muted: self.discreet_muted,
            accent: self.discreet_auxiliary,
            discreet_body: self.discreet_body,
            discreet_muted: self.discreet_muted,
            discreet_auxiliary: self.discreet_auxiliary,
            selected_foreground: self.selected_foreground,
            selected_background: self.selected_background,
            border: self.discreet_muted,
            error: self.error,
            success: self.discreet_auxiliary,
        }
    }

    pub fn body(self) -> Style {
        self.base()
    }

    pub fn muted(self) -> Style {
        self.base().fg(self.muted)
    }

    pub fn accent(self) -> Style {
        self.base().fg(self.accent).add_modifier(Modifier::BOLD)
    }

    pub fn selected(self) -> Style {
        Style::default()
            .fg(self.selected_foreground)
            .bg(self.selected_background)
            .add_modifier(Modifier::BOLD | Modifier::REVERSED)
    }

    pub fn search_match_current(self) -> Style {
        self.selected()
    }

    pub fn search_match_secondary(self) -> Style {
        self.base()
            .fg(self.accent)
            .add_modifier(Modifier::UNDERLINED)
    }

    pub fn popup(self) -> Style {
        self.base().fg(self.body)
    }

    pub fn border(self) -> Style {
        self.base().fg(self.border)
    }

    pub fn error(self) -> Style {
        self.base().fg(self.error).add_modifier(Modifier::BOLD)
    }

    pub fn success(self) -> Style {
        self.base().fg(self.success).add_modifier(Modifier::BOLD)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct KeyChord {
    pub code: KeyCode,
    pub modifiers: KeyModifiers,
}

impl KeyChord {
    pub const fn plain(code: KeyCode) -> Self {
        Self {
            code,
            modifiers: KeyModifiers::NONE,
        }
    }

    pub const fn control(ch: char) -> Self {
        Self {
            code: KeyCode::Char(ch),
            modifiers: KeyModifiers::CONTROL,
        }
    }

    pub fn matches(self, event: KeyEvent) -> bool {
        let mut modifiers = event.modifiers;
        if self.modifiers.is_empty() && matches!(event.code, KeyCode::Char(_)) {
            modifiers.remove(KeyModifiers::SHIFT);
        }
        self.code == event.code && self.modifiers == modifiers
    }
}

#[derive(Debug, Clone, Copy)]
pub struct KeyBinding<C: Copy> {
    pub command: C,
    pub chords: &'static [KeyChord],
    pub keys: &'static str,
    pub hud_keys: &'static str,
    pub label: &'static str,
    pub short_label: &'static str,
    pub show_in_hud: bool,
    pub hud_priority: u8,
}

pub fn command_for<C: Copy>(bindings: &[KeyBinding<C>], event: KeyEvent) -> Option<C> {
    bindings
        .iter()
        .find(|binding| binding.chords.iter().any(|chord| chord.matches(event)))
        .map(|binding| binding.command)
}

pub fn hud_chunks(area: Rect) -> [Rect; 3] {
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(1),
            Constraint::Min(1),
            Constraint::Length(1),
        ])
        .split(area);
    [chunks[0], chunks[1], chunks[2]]
}

pub fn action_line<C: Copy>(
    bindings: &[KeyBinding<C>],
    width: u16,
    palette: Palette,
) -> Line<'static> {
    let visible = bindings
        .iter()
        .filter(|binding| binding.show_in_hud)
        .collect::<Vec<_>>();
    let mut selected = (0..visible.len()).collect::<Vec<_>>();
    let mut mode = if width >= 100 {
        HudMode::Full
    } else if width >= 62 {
        HudMode::Short
    } else {
        HudMode::Keys
    };
    let mut bracket_keys = true;

    fit_hud(&visible, &mut selected, mode, bracket_keys, width);
    if hud_width(&visible, &selected, mode, bracket_keys) > usize::from(width) {
        mode = HudMode::Keys;
        selected = (0..visible.len()).collect();
        fit_hud(&visible, &mut selected, mode, bracket_keys, width);
    }
    if hud_width(&visible, &selected, mode, bracket_keys) > usize::from(width) {
        bracket_keys = false;
        selected = (0..visible.len()).collect();
        fit_hud(&visible, &mut selected, mode, bracket_keys, width);
    }

    let mut spans = Vec::new();

    for (index, &binding_index) in selected.iter().enumerate() {
        let binding = visible[binding_index];
        if index > 0 {
            let separator = if bracket_keys { "  " } else { " " };
            spans.push(Span::styled(separator, palette.muted()));
        }
        let keys = if bracket_keys {
            format!("[{}]", binding.hud_keys)
        } else {
            binding.hud_keys.to_string()
        };
        spans.push(Span::styled(keys, palette.accent()));
        if let Some(label) = hud_label(binding, mode) {
            spans.push(Span::styled(format!(" {label}"), palette.muted()));
        }
    }

    Line::from(spans)
}

#[derive(Clone, Copy)]
enum HudMode {
    Full,
    Short,
    Keys,
}

fn hud_label<C: Copy>(binding: &KeyBinding<C>, mode: HudMode) -> Option<&'static str> {
    match mode {
        HudMode::Full => Some(binding.label),
        HudMode::Short => Some(binding.short_label),
        HudMode::Keys => None,
    }
}

fn fit_hud<C: Copy>(
    bindings: &[&KeyBinding<C>],
    selected: &mut Vec<usize>,
    mode: HudMode,
    bracket_keys: bool,
    width: u16,
) {
    while hud_width(bindings, selected, mode, bracket_keys) > usize::from(width) {
        let Some((position, _)) = selected
            .iter()
            .enumerate()
            .filter(|(_, index)| bindings[**index].hud_priority > 0)
            .max_by_key(|(_, index)| bindings[**index].hud_priority)
        else {
            break;
        };
        selected.remove(position);
    }
}

fn hud_width<C: Copy>(
    bindings: &[&KeyBinding<C>],
    selected: &[usize],
    mode: HudMode,
    bracket_keys: bool,
) -> usize {
    let separator_width = if bracket_keys { 2 } else { 1 };
    selected
        .iter()
        .map(|&index| {
            let binding = bindings[index];
            let key_width = binding.hud_keys.width() + usize::from(bracket_keys) * 2;
            let label_width = hud_label(binding, mode)
                .map(|label| 1 + label.width())
                .unwrap_or(0);
            key_width + label_width
        })
        .sum::<usize>()
        + separator_width * selected.len().saturating_sub(1)
}

pub fn help_lines<C: Copy>(
    heading: &str,
    bindings: &[KeyBinding<C>],
    palette: Palette,
) -> Vec<Line<'static>> {
    let mut lines = vec![Line::from(Span::styled(
        heading.to_string(),
        palette.accent(),
    ))];
    lines.extend(bindings.iter().map(|binding| {
        Line::from(vec![
            Span::styled(format!("  {:<16}", binding.keys), palette.accent()),
            Span::styled(binding.label.to_string(), palette.body()),
        ])
    }));
    lines
}

pub fn centered_rect(percent_x: u16, percent_y: u16, area: Rect) -> Rect {
    let vertical = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Percentage((100 - percent_y) / 2),
            Constraint::Percentage(percent_y),
            Constraint::Percentage((100 - percent_y) / 2),
        ])
        .split(area);
    Layout::default()
        .direction(Direction::Horizontal)
        .constraints([
            Constraint::Percentage((100 - percent_x) / 2),
            Constraint::Percentage(percent_x),
            Constraint::Percentage((100 - percent_x) / 2),
        ])
        .split(vertical[1])[1]
}

pub fn fill_background(frame: &mut ratatui::Frame<'_>, palette: Palette) {
    frame.render_widget(Block::default().style(palette.base()), frame.area());
}

#[cfg(test)]
mod tests {
    use super::*;

    #[derive(Clone, Copy, Debug, PartialEq, Eq)]
    enum Command {
        Help,
    }

    const HELP_KEYS: &[KeyChord] = &[
        KeyChord::plain(KeyCode::Char('?')),
        KeyChord::plain(KeyCode::F(1)),
    ];
    const BINDINGS: &[KeyBinding<Command>] = &[KeyBinding {
        command: Command::Help,
        chords: HELP_KEYS,
        keys: "?/F1",
        hud_keys: "?",
        label: "Full help",
        short_label: "Help",
        show_in_hud: true,
        hud_priority: 0,
    }];

    #[test]
    fn binding_drives_input_and_help_text() {
        let key = KeyEvent::new(KeyCode::Char('?'), KeyModifiers::NONE);
        assert_eq!(command_for(BINDINGS, key), Some(Command::Help));
        let lines = help_lines("Reading", BINDINGS, Palette::terminal());
        assert!(lines[1]
            .spans
            .iter()
            .any(|span| span.content.contains("Full help")));
    }

    #[test]
    fn base_colors_inherit_the_terminal_defaults() {
        let palette = Palette::terminal();
        assert_eq!(palette.background, Color::Reset);
        assert_eq!(palette.body, Color::Reset);
        assert_eq!(palette.selected_background, Color::Reset);
        assert_eq!(palette.selected_foreground, Color::Reset);
        assert!(palette.selected().add_modifier.contains(Modifier::REVERSED));
    }

    #[test]
    fn discreet_mode_keeps_the_terminal_background() {
        let palette = Palette::terminal().discreet_mode();
        assert_eq!(palette.background, Color::Reset);
        assert_eq!(palette.body, Color::Reset);
        assert_eq!(palette.muted, Color::DarkGray);
    }

    #[test]
    fn reading_themes_cycle_all_eight_variants() {
        let mut theme = ReadingTheme::Terminal;
        let expected = [
            ReadingTheme::Sepia,
            ReadingTheme::Eink,
            ReadingTheme::Nord,
            ReadingTheme::Forest,
            ReadingTheme::Dracula,
            ReadingTheme::Rose,
            ReadingTheme::Solarized,
            ReadingTheme::Terminal,
        ];
        for next in expected {
            theme = theme.next();
            assert_eq!(theme, next);
            assert!(!theme.label().is_empty());
            let palette = Palette::for_reading_theme(theme);
            if theme != ReadingTheme::Terminal {
                assert_ne!(palette.body, palette.background);
            } else {
                assert_eq!(palette.body, Color::Reset);
                assert_eq!(palette.background, Color::Reset);
            }
        }
    }

    #[test]
    fn reading_themes_serialize_and_deserialize_lowercase() {
        let themes = [
            (ReadingTheme::Terminal, "\"terminal\""),
            (ReadingTheme::Sepia, "\"sepia\""),
            (ReadingTheme::Eink, "\"eink\""),
            (ReadingTheme::Nord, "\"nord\""),
            (ReadingTheme::Forest, "\"forest\""),
            (ReadingTheme::Dracula, "\"dracula\""),
            (ReadingTheme::Rose, "\"rose\""),
            (ReadingTheme::Solarized, "\"solarized\""),
        ];
        for (theme, json) in themes {
            let serialized = serde_json::to_string(&theme).unwrap();
            assert_eq!(serialized, json);
            let deserialized: ReadingTheme = serde_json::from_str(json).unwrap();
            assert_eq!(deserialized, theme);
        }
    }
}
