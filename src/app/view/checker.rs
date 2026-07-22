//! Scrollable, local-only Harper lint review.

use super::{contextual_tooltip, lucide_icon, ui};
use crate::app::presentation::compact_copy;
use crate::app::{
    BODY_SIZE, CAPTION_SIZE, CHECKER_REVIEW_CLOSE_ID, CHECKER_REVIEW_FIRST_ALWAYS_APPLY_ID,
    CHECKER_REVIEW_FIRST_APPLY_ID, CHECKER_REVIEW_FIRST_IGNORE_ID,
    CHECKER_REVIEW_FIRST_IGNORE_KIND_ID, CHECKER_REVIEW_MODAL_ID, CHECKER_REVIEW_SCROLL_ID,
    CheckerIgnoreScope, CheckerIgnoredLint, CheckerReview, CheckerReviewLint, LEAD_SIZE, Message,
    UI_BOLD_FONT, UI_FONT, UI_SEMIBOLD_FONT,
};
use crate::checker::LintRecord;

use iced::widget::{button, column, container, opaque, row, scrollable, space, text};
use iced::{Center, Element, Fill, Left};
use lucide_icons::Icon;

pub(super) fn modal(review: CheckerReview) -> Element<'static, Message> {
    let applied_count = review.auto_applied.len() + review.manually_applied.len();
    let remaining_count = review.lints.len();
    let ignored_count = review.ignored.len();
    let body = review_body(review);
    let close = container(
        button(
            row![
                lucide_icon(Icon::X, BODY_SIZE, ui::TEXT),
                text("Close").font(UI_FONT).size(BODY_SIZE),
            ]
            .spacing(6)
            .align_y(Center),
        )
        .height(34)
        .padding([6, 12])
        .style(ui::quiet_button)
        .on_press(Message::CloseCheckerReview),
    )
    .id(CHECKER_REVIEW_CLOSE_ID);

    let card = container(
        column![
            row![
                container(lucide_icon(Icon::SpellCheck2, LEAD_SIZE, ui::SUCCESS))
                    .width(34)
                    .height(34)
                    .align_x(Center)
                    .align_y(Center)
                    .style(|_| ui::icon_tile(ui::SUCCESS)),
                column![
                    text("Checker review")
                        .font(UI_BOLD_FONT)
                        .size(LEAD_SIZE)
                        .color(ui::TEXT),
                    text(format!(
                        "{applied_count} applied · {remaining_count} remaining · {ignored_count} ignored"
                    ))
                    .font(UI_FONT)
                    .size(CAPTION_SIZE)
                    .color(ui::SECONDARY),
                ]
                .spacing(2)
                .width(Fill),
                close,
            ]
            .spacing(10)
            .align_y(Center),
            container(space()).width(Fill).height(1).style(ui::rule),
            body,
        ]
        .spacing(12),
    )
    .id(CHECKER_REVIEW_MODAL_ID)
    .width(760)
    .height(Fill)
    .padding(20)
    .style(ui::modal_card);

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

pub(crate) fn tooltip_preview(review: &CheckerReview) -> Element<'static, Message> {
    const SHOWN_PER_GROUP: usize = 3;

    let mut content = column![
        text("Checker review")
            .font(UI_SEMIBOLD_FONT)
            .size(BODY_SIZE)
            .color(ui::TEXT),
        text(format!(
            "{} applied · {} to review · {} ignored",
            review.auto_applied.len() + review.manually_applied.len(),
            review.lints.len(),
            review.ignored.len(),
        ))
        .font(UI_FONT)
        .size(CAPTION_SIZE)
        .color(ui::SECONDARY),
        tooltip_section_label("APPLIED"),
    ]
    .width(360)
    .spacing(5);

    let applied = review
        .auto_applied
        .iter()
        .chain(review.manually_applied.iter())
        .collect::<Vec<_>>();
    if applied.is_empty() {
        content = content.push(tooltip_empty_row());
    } else {
        for lint in applied.iter().take(SHOWN_PER_GROUP) {
            content = content.push(tooltip_lint_row(Icon::Check, ui::SUCCESS, lint));
        }
        if applied.len() > SHOWN_PER_GROUP {
            content = content.push(tooltip_more_row(applied.len() - SHOWN_PER_GROUP));
        }
    }

    content = content.push(tooltip_section_label("TO REVIEW"));
    if review.lints.is_empty() {
        content = content.push(tooltip_empty_row());
    } else {
        for lint in review.lints.iter().take(SHOWN_PER_GROUP) {
            content = content.push(tooltip_lint_row(Icon::AlertCircle, ui::WARNING, &lint.lint));
        }
        if review.lints.len() > SHOWN_PER_GROUP {
            content = content.push(tooltip_more_row(review.lints.len() - SHOWN_PER_GROUP));
        }
    }

    if !review.ignored.is_empty() {
        content = content.push(tooltip_section_label("IGNORED"));
        for ignored in review.ignored.iter().take(SHOWN_PER_GROUP) {
            content = content.push(tooltip_lint_row(Icon::EyeOff, ui::SUBTLE, &ignored.lint));
        }
        if review.ignored.len() > SHOWN_PER_GROUP {
            content = content.push(tooltip_more_row(review.ignored.len() - SHOWN_PER_GROUP));
        }
    }

    content.into()
}

fn tooltip_section_label(label: &'static str) -> Element<'static, Message> {
    text(label)
        .font(UI_SEMIBOLD_FONT)
        .size(CAPTION_SIZE)
        .color(ui::SUBTLE)
        .into()
}

fn tooltip_lint_row(
    icon: Icon,
    color: iced::Color,
    lint: &LintRecord,
) -> Element<'static, Message> {
    row![
        lucide_icon(icon, CAPTION_SIZE, color),
        text(format!("{}", lint.kind))
            .font(UI_SEMIBOLD_FONT)
            .size(CAPTION_SIZE)
            .color(ui::TEXT)
            .width(90),
        text(compact_copy(&lint.message, 58))
            .font(UI_FONT)
            .size(CAPTION_SIZE)
            .color(ui::SECONDARY)
            .width(Fill),
    ]
    .spacing(6)
    .align_y(Center)
    .into()
}

fn tooltip_empty_row() -> Element<'static, Message> {
    text("None")
        .font(UI_FONT)
        .size(CAPTION_SIZE)
        .color(ui::SUBTLE)
        .into()
}

fn tooltip_more_row(count: usize) -> Element<'static, Message> {
    text(format!("+{count} more · open Checker"))
        .font(UI_FONT)
        .size(CAPTION_SIZE)
        .color(ui::ACCENT_TEXT)
        .into()
}

fn review_body(review: CheckerReview) -> Element<'static, Message> {
    let mut content = column![].spacing(10);

    if !review.auto_applied.is_empty() || !review.manually_applied.is_empty() {
        content = content.push(section_label("APPLIED"));
        for lint in review.auto_applied {
            content = content.push(applied_card(lint, "Automatic"));
        }
        for lint in review.manually_applied {
            content = content.push(applied_card(lint, "From review"));
        }
    }

    if !review.ignored.is_empty() {
        content = content.push(section_label("IGNORED · THIS REVIEW"));
        for ignored in review.ignored {
            content = content.push(ignored_card(ignored));
        }
    }

    content = content.push(section_label("TO REVIEW"));
    if review.lints.is_empty() {
        content = content.push(
            container(
                row![
                    lucide_icon(Icon::CheckCircle, LEAD_SIZE, ui::SUCCESS),
                    column![
                        text("All clear")
                            .font(UI_SEMIBOLD_FONT)
                            .size(BODY_SIZE)
                            .color(ui::TEXT),
                        text("No current findings in this dictation context.")
                            .font(UI_FONT)
                            .size(BODY_SIZE)
                            .color(ui::SECONDARY),
                    ]
                    .spacing(2),
                ]
                .spacing(10)
                .align_y(Center),
            )
            .padding(14)
            .style(ui::setting_group),
        );
    } else {
        let mut first_action = true;
        let mut first_always = true;
        let mut first_ignore = true;
        let mut first_ignore_kind = true;
        for (lint_index, lint) in review.lints.into_iter().enumerate() {
            let card = review_card(
                lint,
                lint_index,
                &review.context_text,
                &mut first_action,
                &mut first_always,
                &mut first_ignore,
                &mut first_ignore_kind,
            );
            content = content.push(card);
        }
    }

    scrollable(content)
        .id(CHECKER_REVIEW_SCROLL_ID)
        .height(Fill)
        .into()
}

fn ignored_card(ignored: CheckerIgnoredLint) -> Element<'static, Message> {
    let scope = match ignored.scope {
        CheckerIgnoreScope::Lint => "This lint".to_owned(),
        CheckerIgnoreScope::Kind => format!("All {} lints", ignored.lint.kind),
    };
    container(
        row![
            lucide_icon(Icon::EyeOff, BODY_SIZE, ui::SUBTLE),
            column![
                row![
                    text(format!("{}", ignored.lint.kind))
                        .font(UI_SEMIBOLD_FONT)
                        .size(BODY_SIZE)
                        .color(ui::TEXT),
                    text(scope)
                        .font(UI_FONT)
                        .size(CAPTION_SIZE)
                        .color(ui::SUBTLE),
                ]
                .spacing(8)
                .align_y(Center),
                text(ignored.lint.message)
                    .font(UI_FONT)
                    .size(BODY_SIZE)
                    .color(ui::SECONDARY),
            ]
            .spacing(3)
            .width(Fill),
        ]
        .spacing(10)
        .align_y(Center),
    )
    .padding(12)
    .style(ui::setting_group)
    .into()
}

fn applied_card(lint: LintRecord, source: &'static str) -> Element<'static, Message> {
    container(
        row![
            lucide_icon(Icon::Check, BODY_SIZE, ui::SUCCESS),
            column![
                row![
                    text(format!("{}", lint.kind))
                        .font(UI_SEMIBOLD_FONT)
                        .size(BODY_SIZE)
                        .color(ui::TEXT),
                    text(source)
                        .font(UI_FONT)
                        .size(CAPTION_SIZE)
                        .color(ui::SUCCESS),
                ]
                .spacing(8)
                .align_y(Center),
                text(lint.message)
                    .font(UI_FONT)
                    .size(BODY_SIZE)
                    .color(ui::SECONDARY),
            ]
            .spacing(3)
            .width(Fill),
        ]
        .spacing(10)
        .align_y(Center),
    )
    .padding(12)
    .style(ui::setting_group)
    .into()
}

fn review_card(
    review_lint: CheckerReviewLint,
    lint_index: usize,
    context: &str,
    first_action: &mut bool,
    first_always: &mut bool,
    first_ignore: &mut bool,
    first_ignore_kind: &mut bool,
) -> Element<'static, Message> {
    let lint = review_lint.lint;
    let excerpt = lint_excerpt(context, &lint);
    let status = review_lint.reason.map_or_else(
        || "Review".to_owned(),
        |reason| format!("Skipped · {reason}"),
    );
    let mut actions = column![].spacing(6);

    if lint.suggestions.is_empty() {
        actions = actions.push(
            text("No edit offered")
                .font(UI_FONT)
                .size(CAPTION_SIZE)
                .color(ui::SUBTLE),
        );
    } else {
        for (suggestion_index, suggestion) in lint.suggestions.iter().enumerate() {
            let action = button(
                row![
                    lucide_icon(Icon::Sparkles, BODY_SIZE, ui::ACCENT_TEXT),
                    text(compact_copy(&suggestion.action_label(), 52))
                        .font(UI_FONT)
                        .size(BODY_SIZE),
                ]
                .spacing(6)
                .align_y(Center),
            )
            .height(34)
            .padding([6, 10])
            .style(ui::primary_button)
            .on_press(Message::ApplyCheckerSuggestion {
                lint_index,
                suggestion_index,
            });
            let action: Element<'static, Message> = if *first_action {
                *first_action = false;
                container(action).id(CHECKER_REVIEW_FIRST_APPLY_ID).into()
            } else {
                action.into()
            };
            let always = button(
                container(lucide_icon(Icon::Repeat2, BODY_SIZE, ui::SECONDARY))
                    .width(Fill)
                    .height(Fill)
                    .align_x(Center)
                    .align_y(Center),
            )
            .width(34)
            .height(34)
            .padding(0)
            .style(ui::quiet_button)
            .on_press(Message::AlwaysApplyCheckerSuggestion {
                lint_index,
                suggestion_index,
            });
            let always = contextual_tooltip(
                always,
                "Always apply",
                "Apply this replacement for this review.",
                None,
                None,
                iced::widget::tooltip::Position::Top,
            );
            let always: Element<'static, Message> = if *first_always {
                *first_always = false;
                container(always)
                    .id(CHECKER_REVIEW_FIRST_ALWAYS_APPLY_ID)
                    .into()
            } else {
                always
            };
            actions = actions.push(row![action, always].spacing(6).align_y(Center));
        }
    }

    let ignore_once = button(
        row![
            lucide_icon(Icon::EyeOff, BODY_SIZE, ui::SECONDARY),
            text("Ignore once").font(UI_FONT).size(BODY_SIZE),
        ]
        .spacing(6)
        .align_y(Center),
    )
    .height(34)
    .padding([6, 10])
    .style(ui::quiet_button)
    .on_press(Message::IgnoreCheckerLint { lint_index });
    let ignore_once: Element<'static, Message> = if *first_ignore {
        *first_ignore = false;
        container(ignore_once)
            .id(CHECKER_REVIEW_FIRST_IGNORE_ID)
            .into()
    } else {
        ignore_once.into()
    };
    let ignore_kind = button(
        row![
            lucide_icon(Icon::Tags, BODY_SIZE, ui::SECONDARY),
            text(format!("Ignore {}", lint.kind))
                .font(UI_FONT)
                .size(BODY_SIZE),
        ]
        .spacing(6)
        .align_y(Center),
    )
    .height(34)
    .padding([6, 10])
    .style(ui::quiet_button)
    .on_press(Message::IgnoreCheckerKind { lint_index });
    let ignore_kind: Element<'static, Message> = if *first_ignore_kind {
        *first_ignore_kind = false;
        container(ignore_kind)
            .id(CHECKER_REVIEW_FIRST_IGNORE_KIND_ID)
            .into()
    } else {
        ignore_kind.into()
    };
    actions = actions.push(row![ignore_once, ignore_kind].spacing(6).align_y(Center));

    container(
        column![
            row![
                lucide_icon(Icon::AlertCircle, BODY_SIZE, ui::WARNING),
                text(format!("{}", lint.kind))
                    .font(UI_SEMIBOLD_FONT)
                    .size(BODY_SIZE)
                    .color(ui::TEXT),
                space().width(Fill),
                text(status)
                    .font(UI_FONT)
                    .size(CAPTION_SIZE)
                    .color(ui::WARNING),
            ]
            .spacing(8)
            .align_y(Center),
            text(lint.message)
                .font(UI_FONT)
                .size(BODY_SIZE)
                .color(ui::SECONDARY),
            text(excerpt)
                .font(UI_FONT)
                .size(CAPTION_SIZE)
                .color(ui::SUBTLE),
            actions,
        ]
        .spacing(8)
        .align_x(Left),
    )
    .padding(12)
    .style(ui::setting_group)
    .into()
}

fn section_label(label: &'static str) -> Element<'static, Message> {
    text(label)
        .font(UI_SEMIBOLD_FONT)
        .size(CAPTION_SIZE)
        .color(ui::SUBTLE)
        .into()
}

fn lint_excerpt(context: &str, lint: &LintRecord) -> String {
    const SURROUNDING: usize = 24;
    let chars = context.chars().collect::<Vec<_>>();
    let start = lint.span.start.min(chars.len());
    let end = lint.span.end.min(chars.len()).max(start);
    let excerpt_start = start.saturating_sub(SURROUNDING);
    let excerpt_end = (end + SURROUNDING).min(chars.len());
    let excerpt = chars[excerpt_start..excerpt_end]
        .iter()
        .collect::<String>()
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ");
    format!(
        "{}{}{}",
        if excerpt_start > 0 { "…" } else { "" },
        excerpt,
        if excerpt_end < chars.len() { "…" } else { "" }
    )
}
