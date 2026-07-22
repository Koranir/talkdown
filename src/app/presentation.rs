//! Pure bounded-copy and checker-audit presentation helpers.

use crate::checker::{LintAudit, LintRecord};

pub(super) fn lint_audit_summary(audit: &LintAudit) -> String {
    const SHOWN_PER_DECISION: usize = 3;

    let mut sections = vec![format!(
        "Latest local check: {} applied · {} ignored.",
        audit.fixes(),
        audit.ignored_count()
    )];

    if !audit.applied.is_empty() {
        let records = audit
            .applied
            .iter()
            .take(SHOWN_PER_DECISION)
            .map(lint_record_summary)
            .collect::<Vec<_>>()
            .join("; ");
        sections.push(format!(
            "Applied — {records}{}",
            omitted_suffix(audit.applied.len(), SHOWN_PER_DECISION)
        ));
    }

    if !audit.ignored.is_empty() {
        let records = audit
            .ignored
            .iter()
            .take(SHOWN_PER_DECISION)
            .map(|ignored| {
                format!(
                    "{} ({})",
                    lint_record_summary(&ignored.lint),
                    ignored.reason
                )
            })
            .collect::<Vec<_>>()
            .join("; ");
        sections.push(format!(
            "Ignored — {records}{}",
            omitted_suffix(audit.ignored.len(), SHOWN_PER_DECISION)
        ));
    }

    sections.join("\n")
}

fn lint_record_summary(lint: &LintRecord) -> String {
    let proposal = lint
        .suggestions
        .first()
        .map(|suggestion| format!(" · {suggestion}"))
        .unwrap_or_default();
    format!(
        "{} {}–{}: {}{}",
        lint.kind,
        lint.span.start,
        lint.span.end,
        compact_copy(&lint.message, 96),
        proposal
    )
}

fn omitted_suffix(total: usize, shown: usize) -> String {
    total
        .checked_sub(shown)
        .filter(|omitted| *omitted > 0)
        .map_or_else(String::new, |omitted| {
            format!("; +{omitted} more recorded in memory")
        })
}

pub(super) fn compact_copy(value: &str, max_chars: usize) -> String {
    let normalized = value.split_whitespace().collect::<Vec<_>>().join(" ");
    let mut chars = normalized.chars();
    let prefix: String = chars.by_ref().take(max_chars).collect();

    if chars.next().is_some() {
        format!("{prefix}…")
    } else {
        prefix
    }
}

pub(super) fn compact_tail_copy(value: &str, max_chars: usize) -> String {
    let normalized = value.split_whitespace().collect::<Vec<_>>().join(" ");
    if normalized.chars().count() <= max_chars {
        return normalized;
    }

    let mut tail: Vec<_> = normalized.chars().rev().take(max_chars).collect();
    tail.reverse();
    format!("…{}", tail.into_iter().collect::<String>())
}
