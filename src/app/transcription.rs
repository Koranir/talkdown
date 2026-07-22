//! Pure UTF-8-safe helpers for literal dictation and checker context windows.

use crate::document::DocumentSnapshot;

pub(super) fn transcription_hint(snapshot: &DocumentSnapshot) -> String {
    let range = snapshot.target_range();
    let start = previous_char_boundary(&snapshot.text, range.start.saturating_sub(180));
    let end = next_char_boundary(&snapshot.text, (range.end + 180).min(snapshot.text.len()));
    snapshot.text[start..end].replace('\0', " ")
}

pub(super) fn fit_literal(snapshot: &DocumentSnapshot, transcript: &str) -> String {
    if snapshot.selection.is_some() {
        return transcript.to_owned();
    }

    let previous = snapshot.text[..snapshot.cursor].chars().next_back();
    let next = snapshot.text[snapshot.cursor..].chars().next();
    let first = transcript.chars().next();
    let last = transcript.chars().next_back();
    let prefix =
        previous.is_some_and(char::is_alphanumeric) && first.is_some_and(char::is_alphanumeric);
    let suffix = next.is_some_and(char::is_alphanumeric) && last.is_some_and(char::is_alphanumeric);

    format!(
        "{}{}{}",
        if prefix { " " } else { "" },
        transcript,
        if suffix { " " } else { "" }
    )
}

/// Keeps the local checker fast on large files while giving it enough prose on
/// both sides of a transcript to resolve sentence and agreement boundaries.
/// The returned byte range never begins or ends in the middle of UTF-8 or CRLF.
pub(super) fn harper_context_range(
    text: &str,
    focus: &std::ops::Range<usize>,
) -> std::ops::Range<usize> {
    const CONTEXT_BYTES_PER_SIDE: usize = 512;

    let mut start =
        previous_char_boundary(text, focus.start.saturating_sub(CONTEXT_BYTES_PER_SIDE));
    if start > 0 {
        while start < focus.start {
            let Some(next) = text[start..].chars().next() else {
                break;
            };
            start += next.len_utf8();
            if next.is_whitespace() {
                while start < focus.start
                    && text[start..]
                        .chars()
                        .next()
                        .is_some_and(char::is_whitespace)
                {
                    start += text[start..].chars().next().unwrap().len_utf8();
                }
                break;
            }
        }
    }

    let mut end = next_char_boundary(
        text,
        focus
            .end
            .saturating_add(CONTEXT_BYTES_PER_SIDE)
            .min(text.len()),
    );
    if end < text.len() {
        while end > focus.end {
            let Some(previous) = text[..end].chars().next_back() else {
                break;
            };
            if previous.is_whitespace() {
                break;
            }
            end -= previous.len_utf8();
        }
    }
    if end > 0
        && end < text.len()
        && text.as_bytes()[end - 1] == b'\r'
        && text.as_bytes()[end] == b'\n'
    {
        end += 1;
    }

    start..end
}

pub(super) fn char_offset_to_byte(text: &str, offset: usize) -> Option<usize> {
    if offset == text.chars().count() {
        Some(text.len())
    } else {
        text.char_indices().nth(offset).map(|(byte, _)| byte)
    }
}

pub(super) fn previous_char_boundary(text: &str, mut offset: usize) -> usize {
    while offset > 0 && !text.is_char_boundary(offset) {
        offset -= 1;
    }
    offset
}

pub(super) fn next_char_boundary(text: &str, mut offset: usize) -> usize {
    while offset < text.len() && !text.is_char_boundary(offset) {
        offset += 1;
    }
    offset
}
