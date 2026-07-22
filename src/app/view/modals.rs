//! Opaque safety confirmations layered over the editor workspace.

use super::{fixed_button_label, fixed_icon_label, lucide_icon};
use crate::app::ui;
use crate::app::{
    BODY_SIZE, DISCARD_CONFIRM_ID, DISCARD_KEEP_ID, DISCARD_MODAL_ID, DiscardAction,
    EXTERNAL_CHANGE_KEEP_ID, EXTERNAL_CHANGE_MODAL_ID, EXTERNAL_CHANGE_RELOAD_ID, LEAD_SIZE,
    Message, UI_BOLD_FONT, UI_FONT,
};

use iced::widget::{button, column, container, opaque, row, space, text};
use iced::{Center, Color, Element, Fill};
use lucide_icons::Icon;

pub(super) fn external_file_change(document_name: String) -> Element<'static, Message> {
    let modal = container(
        column![
            modal_header(
                Icon::FileExclamationPoint,
                "File conflict",
                document_name,
                ui::WARNING,
            ),
            modal_rule(),
            prompt_card(
                Icon::Files,
                "Two versions",
                "Your editor copy is unchanged.",
                ui::WARNING,
            ),
            prompt_card(
                Icon::RotateCcw,
                "Reload from disk",
                "Unsaved editor changes will be lost.",
                ui::DANGER,
            ),
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
    let detail = match action {
        DiscardAction::OpenFile => "Canceling Open keeps the current document.",
        DiscardAction::NewFile | DiscardAction::CloseWindow(_) => "This cannot be undone.",
    };
    let modal = container(
        column![
            modal_header(
                Icon::AlertTriangle,
                "Discard changes?",
                document_name,
                ui::WARNING,
            ),
            modal_rule(),
            prompt_card(
                Icon::FileX,
                "Unsaved edits will be lost",
                detail,
                ui::DANGER,
            ),
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
    icon: Icon,
    title: &'static str,
    document_name: String,
    color: Color,
) -> Element<'static, Message> {
    row![
        container(lucide_icon(icon, LEAD_SIZE, color))
            .width(34)
            .height(34)
            .align_x(Center)
            .align_y(Center)
            .style(move |_| ui::icon_tile(color)),
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
    ]
    .spacing(10)
    .align_y(Center)
    .into()
}

fn prompt_card(
    icon: Icon,
    title: &'static str,
    detail: &'static str,
    color: Color,
) -> Element<'static, Message> {
    container(
        row![
            lucide_icon(icon, LEAD_SIZE, color),
            column![
                text(title)
                    .font(UI_BOLD_FONT)
                    .size(BODY_SIZE)
                    .color(ui::TEXT),
                text(detail)
                    .font(UI_FONT)
                    .size(BODY_SIZE)
                    .color(ui::SECONDARY),
            ]
            .spacing(2),
        ]
        .spacing(10)
        .align_y(Center),
    )
    .padding(12)
    .style(ui::setting_group)
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
        button(fixed_button_label("Keep mine", UI_FONT, BODY_SIZE))
            .width(124)
            .height(36)
            .padding([7, 14])
            .style(ui::quiet_button)
            .on_press(Message::KeepExternalEdits),
    )
    .id(EXTERNAL_CHANGE_KEEP_ID);
    let reload = container(
        button(fixed_icon_label(
            Icon::RotateCcw,
            "Reload",
            UI_FONT,
            BODY_SIZE,
        ))
        .width(124)
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
        button(fixed_button_label("Cancel", UI_FONT, BODY_SIZE))
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
