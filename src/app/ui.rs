//! Talkdown Carbon palette and centralized iced widget styles.

use super::UiState;

use iced::widget::{button, container, progress_bar, text_editor, text_input};
use iced::{Background, Border, Color, Shadow, Theme, Vector, theme};

use std::sync::LazyLock;

pub const WINDOW: Color = Color::from_rgb8(0x15, 0x15, 0x15);
pub const EDITOR: Color = Color::from_rgb8(0x19, 0x19, 0x19);
pub const SURFACE: Color = Color::from_rgb8(0x22, 0x22, 0x22);
pub const SURFACE_HOVER: Color = Color::from_rgb8(0x2B, 0x2B, 0x2B);
pub const BORDER: Color = Color::from_rgb8(0x41, 0x41, 0x41);
pub const BORDER_STRONG: Color = Color::from_rgb8(0x5A, 0x5A, 0x5A);
pub const TEXT: Color = Color::from_rgb8(0xC9, 0xC9, 0xC9);
pub const SECONDARY: Color = Color::from_rgb8(0x99, 0x99, 0x99);
pub const SUBTLE: Color = Color::from_rgb8(0x8C, 0x8C, 0x8C);
pub const PRIMARY: Color = Color::from_rgb8(0xFF, 0x00, 0x95);
pub const PRIMARY_HOVER: Color = Color::from_rgb8(0xFF, 0x2E, 0xAA);
pub const PRIMARY_PRESSED: Color = Color::from_rgb8(0xD9, 0x00, 0x80);
pub const VOICE: Color = PRIMARY;
pub const SUCCESS: Color = Color::from_rgb8(0x78, 0xBD, 0x9B);
pub const WARNING: Color = Color::from_rgb8(0xDF, 0xB2, 0x68);
pub const DANGER: Color = Color::from_rgb8(0xF0, 0x70, 0x80);
pub const OFFLINE: Color = Color::from_rgb8(0x8C, 0x8C, 0x8C);

pub const INFO_SURFACE: Color = Color::from_rgb8(0x26, 0x00, 0x0F);
pub const VOICE_SURFACE: Color = Color::from_rgb8(0x26, 0x00, 0x0F);
pub const SUCCESS_SURFACE: Color = Color::from_rgb8(0x19, 0x24, 0x1E);
pub const WARNING_SURFACE: Color = Color::from_rgb8(0x2A, 0x22, 0x18);
pub const DANGER_SURFACE: Color = Color::from_rgb8(0x2B, 0x19, 0x1D);
pub const WINE_HOVER: Color = Color::from_rgb8(0x33, 0x00, 0x15);
pub const WINE_PRESSED: Color = Color::from_rgb8(0x1B, 0x00, 0x0B);
pub const ACCENT_TEXT: Color = Color::from_rgb8(0xFF, 0x5D, 0xB5);
const DISABLED: Color = Color::from_rgb8(0x70, 0x70, 0x70);
const FOCUS_BORDER: Color = Color::from_rgb8(0x7A, 0x35, 0x5A);

pub static THEME: LazyLock<Theme> = LazyLock::new(|| {
    Theme::custom(
        "Talkdown Carbon",
        theme::palette::Seed {
            background: WINDOW,
            text: TEXT,
            primary: PRIMARY,
            success: SUCCESS,
            warning: WARNING,
            danger: DANGER,
        },
    )
});

pub fn shell(_: &Theme) -> container::Style {
    container::Style::default().background(WINDOW)
}

pub fn raised(_: &Theme) -> container::Style {
    container::Style::default()
        .background(SURFACE)
        .border(Border::default().rounded(12).width(1).color(BORDER))
        .shadow(Shadow {
            color: Color::BLACK.scale_alpha(0.42),
            offset: Vector::new(0.0, 5.0),
            blur_radius: 16.0,
        })
}

pub fn tooltip(_: &Theme) -> container::Style {
    container::Style::default()
        .background(SURFACE_HOVER)
        .border(Border::default().rounded(8).width(1).color(BORDER_STRONG))
        .shadow(Shadow {
            color: Color::BLACK.scale_alpha(0.58),
            offset: Vector::new(0.0, 5.0),
            blur_radius: 18.0,
        })
}

pub fn modal_backdrop(_: &Theme) -> container::Style {
    container::Style::default().background(Color::BLACK.scale_alpha(0.72))
}

pub fn modal_card(_: &Theme) -> container::Style {
    container::Style::default()
        .background(SURFACE)
        .border(Border::default().rounded(14).width(1).color(BORDER_STRONG))
        .shadow(Shadow {
            color: Color::BLACK.scale_alpha(0.72),
            offset: Vector::new(0.0, 12.0),
            blur_radius: 32.0,
        })
}

pub fn setting_group(_: &Theme) -> container::Style {
    container::Style::default()
        .background(EDITOR)
        .border(Border::default().rounded(10).width(1).color(BORDER))
}

pub fn rule(_: &Theme) -> container::Style {
    container::Style::default().background(BORDER)
}

pub fn accent_rule(_: &Theme) -> container::Style {
    container::Style::default().background(PRIMARY)
}

pub fn mode_pill(color: Color) -> container::Style {
    container::Style::default()
        .background(color.scale_alpha(0.14))
        .border(
            Border::default()
                .rounded(6)
                .width(1)
                .color(color.scale_alpha(0.5)),
        )
}

pub fn status_pill(color: Color) -> container::Style {
    container::Style::default()
        .background(color.scale_alpha(0.12))
        .border(
            Border::default()
                .rounded(99)
                .width(1)
                .color(color.scale_alpha(0.34)),
        )
}

pub fn notice(state: UiState) -> container::Style {
    let (background, accent) = match state {
        UiState::Success | UiState::Ready => (SUCCESS_SURFACE, SUCCESS),
        UiState::Warning => (WARNING_SURFACE, WARNING),
        UiState::Error => (DANGER_SURFACE, DANGER),
        UiState::Offline => (DANGER_SURFACE, DANGER),
        UiState::Listening => (VOICE_SURFACE, VOICE),
        UiState::Info | UiState::Working => (INFO_SURFACE, PRIMARY),
    };

    container::Style::default().background(background).border(
        Border::default()
            .rounded(8)
            .width(1)
            .color(accent.scale_alpha(0.62)),
    )
}

pub fn quiet_button(_: &Theme, status: button::Status) -> button::Style {
    let (background, text_color, border_color) = match status {
        button::Status::Active => (SURFACE, SECONDARY, BORDER),
        button::Status::Hovered => (SURFACE_HOVER, TEXT, BORDER_STRONG),
        button::Status::Pressed => (EDITOR, TEXT, PRIMARY),
        button::Status::Disabled => (SURFACE, DISABLED, BORDER.scale_alpha(0.8)),
    };

    button::Style {
        background: Some(Background::Color(background)),
        text_color,
        border: Border::default().rounded(6).width(1).color(border_color),
        ..button::Style::default()
    }
}

pub fn primary_button(_: &Theme, status: button::Status) -> button::Style {
    let (background, text_color, border_color) = match status {
        button::Status::Active => (INFO_SURFACE, ACCENT_TEXT, PRIMARY),
        button::Status::Hovered => (WINE_HOVER, PRIMARY_HOVER, PRIMARY_HOVER),
        button::Status::Pressed => (WINE_PRESSED, ACCENT_TEXT, PRIMARY_PRESSED),
        button::Status::Disabled => (SURFACE, DISABLED, BORDER),
    };

    button::Style {
        background: Some(Background::Color(background)),
        text_color,
        border: Border::default().rounded(6).width(1).color(border_color),
        shadow: Shadow {
            color: PRIMARY.scale_alpha(0.1),
            offset: Vector::new(0.0, 2.0),
            blur_radius: 8.0,
        },
        ..button::Style::default()
    }
}

pub fn danger_button(_: &Theme, status: button::Status) -> button::Style {
    let (background, text_color, border_color) = match status {
        button::Status::Active => (DANGER_SURFACE, DANGER, DANGER.scale_alpha(0.72)),
        button::Status::Hovered => (DANGER.scale_alpha(0.18), TEXT, DANGER),
        button::Status::Pressed => (DANGER.scale_alpha(0.1), DANGER, DANGER),
        button::Status::Disabled => (SURFACE, DISABLED, BORDER),
    };

    button::Style {
        background: Some(Background::Color(background)),
        text_color,
        border: Border::default().rounded(6).width(1).color(border_color),
        shadow: Shadow {
            color: DANGER.scale_alpha(0.08),
            offset: Vector::new(0.0, 2.0),
            blur_radius: 8.0,
        },
        ..button::Style::default()
    }
}

pub fn editor(theme: &Theme, status: text_editor::Status) -> text_editor::Style {
    let mut style = text_editor::default(theme, status);
    style.background = Background::Color(EDITOR);
    style.value = TEXT;
    style.placeholder = SUBTLE;
    style.selection = PRIMARY.scale_alpha(0.42);
    style.border = Border::default().rounded(12).width(1).color(
        if matches!(status, text_editor::Status::Focused { .. }) {
            FOCUS_BORDER
        } else if matches!(status, text_editor::Status::Hovered) {
            BORDER_STRONG
        } else {
            BORDER
        },
    );
    style
}

pub fn command_input(theme: &Theme, status: text_input::Status) -> text_input::Style {
    let mut style = text_input::default(theme, status);
    style.background = Background::Color(EDITOR);
    style.value = TEXT;
    style.placeholder = SUBTLE;
    style.selection = PRIMARY.scale_alpha(0.42);
    style.border = Border::default()
        .rounded(8)
        .width(if matches!(status, text_input::Status::Focused { .. }) {
            2
        } else {
            1
        })
        .color(if matches!(status, text_input::Status::Focused { .. }) {
            WARNING
        } else {
            BORDER
        });
    style
}

pub fn meter(_: &Theme) -> progress_bar::Style {
    progress_bar::Style {
        background: Background::Color(SURFACE_HOVER),
        bar: Background::Color(VOICE),
        border: Border::default().rounded(99),
    }
}
