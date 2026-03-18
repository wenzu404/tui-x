use ratatui::style::{Color, Modifier, Style};

/// X/Twitter brand colors and UI theme.
pub struct Theme;

impl Theme {
    // Brand colors
    pub const BLUE: Color = Color::Rgb(29, 155, 240);
    pub const WHITE: Color = Color::Rgb(231, 233, 234);
    pub const GRAY: Color = Color::Rgb(113, 118, 123);
    pub const DARK_GRAY: Color = Color::Rgb(47, 51, 54);
    pub const BG: Color = Color::Rgb(0, 0, 0);
    pub const RED: Color = Color::Rgb(249, 24, 128);   // Like heart
    pub const GREEN: Color = Color::Rgb(0, 186, 124);   // Retweet
    pub const YELLOW: Color = Color::Rgb(255, 212, 0);  // Bookmark

    // Styles
    pub fn text() -> Style {
        Style::default().fg(Self::WHITE)
    }

    pub fn dimmed() -> Style {
        Style::default().fg(Self::GRAY)
    }

    pub fn accent() -> Style {
        Style::default().fg(Self::BLUE)
    }

    pub fn bold() -> Style {
        Style::default()
            .fg(Self::WHITE)
            .add_modifier(Modifier::BOLD)
    }

    pub fn username() -> Style {
        Style::default()
            .fg(Self::BLUE)
            .add_modifier(Modifier::BOLD)
    }

    pub fn handle() -> Style {
        Style::default().fg(Self::GRAY)
    }

    pub fn like() -> Style {
        Style::default().fg(Self::RED)
    }

    pub fn retweet() -> Style {
        Style::default().fg(Self::GREEN)
    }

    pub fn bookmark() -> Style {
        Style::default().fg(Self::YELLOW)
    }

    pub fn selected() -> Style {
        Style::default()
            .fg(Self::WHITE)
            .bg(Self::DARK_GRAY)
    }

    pub fn tab_active() -> Style {
        Style::default()
            .fg(Self::BLUE)
            .add_modifier(Modifier::BOLD | Modifier::UNDERLINED)
    }

    pub fn tab_inactive() -> Style {
        Style::default().fg(Self::GRAY)
    }

    pub fn border() -> Style {
        Style::default().fg(Self::DARK_GRAY)
    }
}
