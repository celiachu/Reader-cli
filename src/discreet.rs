//! Agent-style presentation for local reading. Legacy diagnostic exports remain compatible.
use crate::book::{Section, TextPosition};
use crate::pagination::{wrap_text_with_layout, AnchoredLine};
use ratatui::layout::Rect;

pub const DISCREET_ERROR_LINE: &str =
    "TypeError: Cannot read properties of undefined (reading 'profile')";

#[cfg(windows)]
const CAMOUFLAGE_STACK: &str = r#"PS C:\workspace\dashboard-client> node .\dist\server.js

C:\workspace\dashboard-client\dist\chunks\session.js:184
  const profile = state.session.profile;
                                ^

TypeError: Cannot read properties of undefined (reading 'profile')
    at resolveActiveSession (dist\chunks\session.js:184:17)
    at renderWorkspace (dist\chunks\workspace.js:221:9)
    at commitHookEffectListMount (node_modules\react-dom\cjs\react-dom.production.min.js:13:421)
    at commitPassiveMountOnFiber (node_modules\react-dom\cjs\react-dom.production.min.js:14:318)
    at recursivelyTraversePassiveMountEffects (node_modules\react-dom\cjs\react-dom.production.min.js:14:189)
    at commitPassiveMountEffects (node_modules\react-dom\cjs\react-dom.production.min.js:15:732)
    at flushPassiveEffectsImpl (node_modules\react-dom\cjs\react-dom.production.min.js:16:214)
    at flushPassiveEffects (node_modules\react-dom\cjs\react-dom.production.min.js:16:91)
    at performSyncWorkOnRoot (node_modules\react-dom\cjs\react-dom.production.min.js:19:311)
    at flushSyncCallbacks (node_modules\react-dom\cjs\react-dom.production.min.js:19:142)
    at MessagePort.performWorkUntilDeadline (node_modules\scheduler\cjs\scheduler.production.min.js:7:412)

PS C:\workspace\dashboard-client>"#;

#[cfg(not(windows))]
const CAMOUFLAGE_STACK: &str = r#"~/workspace/dashboard-client $ node ./dist/server.js

/workspace/dashboard-client/dist/chunks/session.js:184
  const profile = state.session.profile;
                                ^

TypeError: Cannot read properties of undefined (reading 'profile')
    at resolveActiveSession (dist/chunks/session.js:184:17)
    at renderWorkspace (dist/chunks/workspace.js:221:9)
    at commitHookEffectListMount (node_modules/react-dom/cjs/react-dom.production.min.js:13:421)
    at commitPassiveMountOnFiber (node_modules/react-dom/cjs/react-dom.production.min.js:14:318)
    at recursivelyTraversePassiveMountEffects (node_modules/react-dom/cjs/react-dom.production.min.js:14:189)
    at commitPassiveMountEffects (node_modules/react-dom/cjs/react-dom.production.min.js:15:732)
    at flushPassiveEffectsImpl (node_modules/react-dom/cjs/react-dom.production.min.js:16:214)
    at flushPassiveEffects (node_modules/react-dom/cjs/react-dom.production.min.js:16:91)
    at performSyncWorkOnRoot (node_modules/react-dom/cjs/react-dom.production.min.js:19:311)
    at flushSyncCallbacks (node_modules/react-dom/cjs/react-dom.production.min.js:19:142)
    at MessagePort.performWorkUntilDeadline (node_modules/scheduler/cjs/scheduler.production.min.js:7:412)

~/workspace/dashboard-client $"#;

pub fn discreet_stack() -> &'static str {
    CAMOUFLAGE_STACK
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub(crate) enum AgentDensity {
    #[default]
    Comfortable,
    Inspect,
}

impl AgentDensity {
    pub fn toggle(self) -> Self {
        match self {
            Self::Comfortable => Self::Inspect,
            Self::Inspect => Self::Comfortable,
        }
    }

    #[allow(dead_code)]
    pub fn label(self) -> &'static str {
        match self {
            Self::Comfortable => "comfortable",
            Self::Inspect => "inspect",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum AgentViewMode {
    Full,
    Compact,
}

#[derive(Debug, Clone, Copy)]
pub(crate) struct AgentLayout {
    pub mode: AgentViewMode,
    pub banner: Rect,
    pub header: Rect,
    pub body: Rect,
    pub footer: Rect,
}

impl AgentLayout {
    #[allow(dead_code)]
    pub fn new(area: Rect, configured_width: Option<u16>) -> Self {
        Self::with_options(area, configured_width, None)
    }

    pub fn with_options(
        area: Rect,
        configured_width: Option<u16>,
        custom_agent_lines: Option<u16>,
    ) -> Self {
        if area.height < 6 {
            return Self {
                mode: AgentViewMode::Compact,
                banner: Rect::default(),
                header: Rect::new(area.x, area.y, area.width, area.height.min(1)),
                body: Rect::default(),
                footer: Rect::default(),
            };
        }
        let is_full = area.width >= 72 && area.height >= 22;
        let mode = if is_full {
            AgentViewMode::Full
        } else {
            AgentViewMode::Compact
        };

        let foot_h = if is_full {
            3
        } else if area.height >= 14 {
            2
        } else {
            1
        };

        let (banner_h, head_h, body_height) = if let Some(lines) = custom_agent_lines {
            // User explicitly customized the simulated agent rows.
            let banner_h = if is_full && area.height >= 14 {
                3.min(area.height.saturating_sub(foot_h + 3))
            } else {
                0
            };
            let max_possible_head = area.height.saturating_sub(banner_h + foot_h + 3);
            let head_h = lines.min(max_possible_head).max(1);
            let body_h = area
                .height
                .saturating_sub(banner_h + head_h + foot_h)
                .max(3);
            (banner_h, head_h, body_h)
        } else {
            // 60:40 Rule: Visible prose is capped at <= 40% of viewport height.
            let max_text = match area.height {
                0..=12 => 3,
                13..=19 => 5,
                20..=25 => 7,
                26..=32 => 9,
                _ => 12,
            };
            let body_height = (max_text + 2).min(area.height.saturating_sub(4));
            let foot_h = if is_full {
                3
            } else {
                2.min(area.height.saturating_sub(body_height))
            };
            let remaining = area.height.saturating_sub(body_height + foot_h);
            let banner_h = if is_full { 3.min(remaining) } else { 0 };
            let head_h = remaining.saturating_sub(banner_h);
            (banner_h, head_h, body_height)
        };

        let indent = if area.width < 50 { 2 } else { 4 };
        let available = area.width.saturating_sub(indent + 2);
        let body_width = configured_width.unwrap_or(72).max(20).min(available);

        Self {
            mode,
            banner: Rect::new(area.x, area.y, area.width, banner_h),
            header: Rect::new(area.x, area.y + banner_h, area.width, head_h),
            body: Rect::new(
                area.x + indent.min(area.width),
                area.y + banner_h + head_h,
                body_width,
                body_height,
            ),
            footer: Rect::new(
                area.x,
                area.y + banner_h + head_h + body_height,
                area.width,
                foot_h,
            ),
        }
    }

    /// Net width available for text based on density.
    /// Comfortable: full body width with 2 columns margin.
    /// Inspect: retains space for line number (7 cols: `NNN │ `).
    pub fn net_text_width(&self, density: AgentDensity) -> u16 {
        match density {
            AgentDensity::Comfortable => self.body.width.saturating_sub(2).max(20),
            AgentDensity::Inspect => self.body.width.saturating_sub(7).max(20),
        }
    }

    /// Net visible lines available for verbatim prose.
    /// Subtracts the "● Assistant" and "[Output complete]" marker rows.
    pub fn net_text_height(&self) -> u16 {
        self.body.height.saturating_sub(2).max(1)
    }
}

#[derive(Debug)]
pub(crate) struct ContextLine {
    pub section: usize,
    pub line: AnchoredLine,
}

impl ContextLine {
    pub fn position(&self) -> TextPosition {
        TextPosition::new(self.section, self.line.start_char)
    }
}

#[derive(Debug, Default)]
pub(crate) struct AgentViewport {
    pub lines: Vec<ContextLine>,
    pub top: usize,
    pub context_rows: usize,
    width: u16,
}

impl AgentViewport {
    pub fn reflow(&mut self, sections: &[Section], width: u16, position: TextPosition) {
        if self.width != width || self.lines.is_empty() {
            self.lines = sections
                .iter()
                .enumerate()
                .flat_map(|(section, source)| {
                    wrap_text_with_layout(&source.text, usize::from(width.max(1)), true)
                        .into_iter()
                        .map(move |line| ContextLine { section, line })
                })
                .collect();
            self.width = width;
        }
        let anchor = self
            .lines
            .iter()
            .rposition(|line| line.position() <= position)
            .unwrap_or(0);
        self.context_rows = usize::from(
            self.context_rows > 0
                && anchor > 0
                && self.lines[anchor - 1].section == self.lines[anchor].section,
        );
        self.top = anchor.saturating_sub(self.context_rows);
    }

    pub fn visible(&self, height: u16) -> &[ContextLine] {
        let start = self.top.min(self.lines.len());
        &self.lines[start
            ..start
                .saturating_add(usize::from(height))
                .min(self.lines.len())]
    }
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub(crate) enum ReadDirection {
    #[default]
    Current,
    Forward,
    Backward,
}

/// A bounded summary of real operations, never copies of tool-output text.
#[derive(Debug, Default)]
pub(crate) struct AgentSession {
    pub view: AgentViewport,
    pub reads: usize,
    pub resumed: bool,
    pub direction: ReadDirection,
    pub density: AgentDensity,
    pub command: Option<String>,
    pub help: bool,
    pub help_offset: usize,
    pub receipt: Option<&'static str>,
    pub checkpoints: usize,
}

impl AgentSession {
    pub fn record_read(&mut self, direction: ReadDirection) {
        self.reads = self.reads.saturating_add(1);
        self.direction = direction;
        self.receipt = None;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn layout_matches_approved_grid_and_never_escapes_viewport() {
        for (w, h, mode, banner, head, body, foot, cols, net_text) in [
            (80, 24, AgentViewMode::Full, 3, 9, 9, 3, 72, 7),
            (120, 30, AgentViewMode::Full, 3, 13, 11, 3, 72, 9),
            (60, 18, AgentViewMode::Compact, 0, 9, 7, 2, 54, 5),
        ] {
            let l = AgentLayout::new(Rect::new(0, 0, w, h), None);
            assert_eq!(
                (
                    l.mode,
                    l.banner.height,
                    l.header.height,
                    l.body.height,
                    l.footer.height,
                    l.body.width,
                    l.net_text_height()
                ),
                (mode, banner, head, body, foot, cols, net_text)
            );
            // 60:40 verification: prose <= 40% of height, Agent >= 50%
            assert!(f64::from(l.net_text_height()) / f64::from(h) <= 0.40);
            assert!(
                f64::from(l.banner.height + l.header.height + l.footer.height) / f64::from(h)
                    >= 0.50
            );
        }
        for w in 0..=140 {
            for h in 0..=40 {
                let l = AgentLayout::new(Rect::new(0, 0, w, h), Some(100));
                for area in [l.banner, l.header, l.body, l.footer] {
                    assert!(area.right() <= w && area.bottom() <= h);
                }
            }
        }
    }

    #[test]
    fn layout_with_custom_agent_lines_and_high_terminal() {
        // High terminal: 120x45, default dynamic calculation allocates 25 rows for header
        let default_high = AgentLayout::new(Rect::new(0, 0, 120, 45), None);
        assert_eq!(default_high.header.height, 25);
        assert_eq!(default_high.banner.height, 3);
        assert_eq!(default_high.footer.height, 3);
        assert_eq!(default_high.body.height, 14);

        // Custom agent lines: request 18 rows
        let custom_18 = AgentLayout::with_options(Rect::new(0, 0, 120, 45), None, Some(18));
        assert_eq!(custom_18.header.height, 18);
        assert_eq!(custom_18.body.height, 21);

        // Custom agent lines on compact high terminal: 60x40
        let compact_high = AgentLayout::with_options(Rect::new(0, 0, 60, 40), None, Some(20));
        assert_eq!(compact_high.header.height, 20);
        assert_eq!(compact_high.footer.height, 2);
        assert_eq!(compact_high.body.height, 18);
    }
}
