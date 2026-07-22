//! Pure bounded-copy and checker-audit presentation helpers.

use crate::checker::LintAudit;

pub(super) fn lint_audit_summary(audit: &LintAudit) -> String {
    format!(
        "Latest check · {} applied · {} ignored. Click to inspect.",
        audit.fixes(),
        audit.ignored_count()
    )
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
