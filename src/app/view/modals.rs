//! Opaque safety confirmations layered over the editor workspace.

use super::fixed_button_label;
use crate::app::ui;
use crate::app::{
    BODY_SIZE, CAPTION_SIZE, DISCARD_CONFIRM_ID, DISCARD_KEEP_ID, DISCARD_MODAL_ID, DiscardAction,
    EDITOR_FONT, EXTERNAL_CHANGE_KEEP_ID, EXTERNAL_CHANGE_MODAL_ID, EXTERNAL_CHANGE_RELOAD_ID,
    LEAD_SIZE, Message, UI_BOLD_FONT, UI_FONT,
};

use iced::widget::{button, column, container, opaque, row, space, text};
use iced::{Center, Color, Element, Fill};

pub(super) fn external_file_change(document_name: String) -> Element<'static, Message> {
    let modal = container(
        column![
            modal_header(
                "File changed on disk",
                document_name,
                "CONFLICT",
                ui::WARNING,
            ),
            modal_rule(),
            text("Talkdown found a different disk version while the editor has unsaved changes. Your editor text has not been changed.")
                .font(UI_FONT)
                .size(BODY_SIZE)
                .line_height(1.35)
                .color(ui::SECONDARY),
            text("Reloading discards the unsaved editor changes and cannot be undone.")
                .font(UI_FONT)
                .size(BODY_SIZE)
                .line_height(1.35)
                .color(ui::DANGER),
            external_file_actions(),
        ]
        .spacing(14),
    )
    .id(EXTERNAL_CHANGE_MODAL_ID)
    .width(560)
    .padding(20)
    .style(ui::modal_card);

    modal_backdrop(modal)
}

pub(super) fn discard_changes(
    action: DiscardAction,
    document_name: String,
) -> Element<'static, Message> {
    let consequence = match action {
        DiscardAction::OpenFile => {
            "Your current buffer stays intact if the picker is cancelled or the selected file cannot be opened."
        }
        DiscardAction::NewFile | DiscardAction::CloseWindow(_) => {
            "This cannot be undone after you continue."
        }
    };
    let modal = container(
        column![
            modal_header(
                "Discard unsaved changes?",
                document_name,
                "UNSAVED",
                ui::WARNING,
            ),
            modal_rule(),
            text(format!(
                "If you {}, changes that have not been saved will be discarded.",
                action.verb()
            ))
            .font(UI_FONT)
            .size(BODY_SIZE)
            .line_height(1.35)
            .color(ui::SECONDARY),
            text(consequence)
                .font(UI_FONT)
                .size(BODY_SIZE)
                .line_height(1.35)
                .color(ui::DANGER),
            discard_actions(action),
        ]
        .spacing(14),
    )
    .id(DISCARD_MODAL_ID)
    .width(540)
    .padding(20)
    .style(ui::modal_card);

    modal_backdrop(modal)
}

fn modal_header(
    title: &'static str,
    document_name: String,
    badge: &'static str,
    color: Color,
) -> Element<'static, Message> {
    row![
        column![
            text(title)
                .font(UI_BOLD_FONT)
                .size(LEAD_SIZE)
                .color(ui::TEXT),
            text(document_name)
                .font(UI_FONT)
                .size(BODY_SIZE)
                .color(ui::SUBTLE),
        ]
        .spacing(2)
        .width(Fill),
        container(
            text(badge)
                .font(EDITOR_FONT)
                .size(CAPTION_SIZE)
                .color(color),
        )
        .padding([5, 8])
        .style(move |_| ui::status_pill(color)),
    ]
    .align_y(Center)
    .into()
}

fn modal_rule() -> Element<'static, Message> {
    container(space())
        .width(Fill)
        .height(1)
        .style(ui::rule)
        .into()
}

fn external_file_actions() -> Element<'static, Message> {
    let keep = container(
        button(fixed_button_label("Keep editing", UI_FONT, BODY_SIZE))
            .width(124)
            .height(36)
            .padding([7, 14])
            .style(ui::quiet_button)
            .on_press(Message::KeepExternalEdits),
    )
    .id(EXTERNAL_CHANGE_KEEP_ID);
    let reload = container(
        button(fixed_button_label("Reload from disk", UI_FONT, BODY_SIZE))
            .width(154)
            .height(36)
            .padding([7, 14])
            .style(ui::danger_button)
            .on_press(Message::ReloadExternalFile),
    )
    .id(EXTERNAL_CHANGE_RELOAD_ID);

    row![space().width(Fill), keep, reload]
        .spacing(8)
        .align_y(Center)
        .into()
}

fn discard_actions(action: DiscardAction) -> Element<'static, Message> {
    let keep = container(
        button(fixed_button_label("Keep editing", UI_FONT, BODY_SIZE))
            .width(124)
            .height(36)
            .padding([7, 14])
            .style(ui::quiet_button)
            .on_press(Message::CancelDiscard),
    )
    .id(DISCARD_KEEP_ID);
    let discard = container(
        button(fixed_button_label(
            action.button_label(),
            UI_FONT,
            BODY_SIZE,
        ))
        .width(144)
        .height(36)
        .padding([7, 14])
        .style(ui::danger_button)
        .on_press(Message::ConfirmDiscard),
    )
    .id(DISCARD_CONFIRM_ID);

    row![space().width(Fill), keep, discard]
        .spacing(8)
        .align_y(Center)
        .into()
}

fn modal_backdrop(card: impl Into<Element<'static, Message>>) -> Element<'static, Message> {
    opaque(
        container(card)
            .width(Fill)
            .height(Fill)
            .align_x(Center)
            .align_y(Center)
            .padding(24)
            .style(ui::modal_backdrop),
    )
}
