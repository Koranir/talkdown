//! Bounded transcript checking and locally validated Codex edit transactions.

use super::presentation::{compact_copy, lint_audit_summary};
use super::transcription::{
    char_offset_to_byte, fit_literal, harper_context_range, next_char_boundary,
};
use super::{
    App, CheckerIgnoreScope, CheckerIgnoredLint, CheckerReview, CheckerReviewLint, Message, Notice,
    NoticeSource, PendingEdit, UiState,
};

use crate::checker::{CheckResult, CheckingProvider, IgnoreReason};
use crate::codex::{CodexEvent, CodexRequest, CodexSubmitError, editable_context_range};
use crate::document::DocumentSnapshot;
use crate::edit::{Anchor, EditIntent, ProposedEdit, rebase_exact, resolve};

use std::ffi;
use std::ops::Range;
use std::path::Path;

use iced::Task;
use iced::widget::operation;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum CodexFailureScope {
    Service,
    Background,
    Current(EditIntent),
}

struct PlacedTranscript {
    range: Range<usize>,
    raw: String,
    changed: bool,
}

struct HarperCheckPlan {
    context_range: Range<usize>,
    original_context: String,
    focus: Range<usize>,
}

struct PreparedCodexSubmission {
    request_id: u64,
    request: CodexRequest,
    pending: PendingEdit,
}

struct ResolvedCodexProposal {
    range: Range<usize>,
    replacement: String,
    summary: String,
}

enum CodexEditOutcome {
    Applied { summary: String },
    Rejected(CodexEditRejection),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum CodexEditRejection {
    PreviousDocument,
    UnsafeRefinement,
    UnresolvedTarget,
    OutsideSharedContext,
    StaleInsertion,
    MissingStaleTarget,
    AmbiguousStaleTarget,
    DocumentRejected,
}

struct CodexRejectionNotice {
    title: &'static str,
    detail: &'static str,
    recovery: &'static str,
}

impl CodexEditRejection {
    fn notice(self) -> CodexRejectionNotice {
        match self {
            Self::PreviousDocument => CodexRejectionNotice {
                title: "Edit ignored for a previous document",
                detail: "The response belonged to a buffer that is no longer open.",
                recovery: "The current document was not changed.",
            },
            Self::UnsafeRefinement => CodexRejectionNotice {
                title: "Refinement skipped for safety",
                detail: "Codex attempted to leave the exact dictated span.",
                recovery: "Codex applied no replacement and did not roll back later local edits.",
            },
            Self::UnresolvedTarget => CodexRejectionNotice {
                title: "Edit skipped for safety",
                detail: "The proposed target did not resolve exactly near the captured cursor.",
                recovery: "No Codex change was applied.",
            },
            Self::OutsideSharedContext => CodexRejectionNotice {
                title: "Edit skipped outside the shared context",
                detail: "The proposed target extended beyond the cursor window shown to Codex.",
                recovery: "No Codex change was applied.",
            },
            Self::StaleInsertion => CodexRejectionNotice {
                title: "Stale insertion skipped safely",
                detail: "The cursor moved after the command was captured.",
                recovery: "No text was inserted; repeat the command at the new cursor.",
            },
            Self::MissingStaleTarget => CodexRejectionNotice {
                title: "Stale edit skipped safely",
                detail: "The exact target disappeared while Codex was working.",
                recovery: "No Codex change was applied; repeat the command if it is still needed.",
            },
            Self::AmbiguousStaleTarget => CodexRejectionNotice {
                title: "Ambiguous edit skipped safely",
                detail: "The target now appears more than once, so Talkdown refused to guess.",
                recovery: "No Codex change was applied; select the intended text and try again.",
            },
            Self::DocumentRejected => CodexRejectionNotice {
                title: "Edit failed local validation",
                detail: "The final replacement range was rejected by the document model.",
                recovery: "No unsafe replacement was applied.",
            },
        }
    }
}

impl App {
    fn reject_latest_harper_audit(&mut self, reason: IgnoreReason) {
        if let Some(mut audit) = self.last_harper_audit.take() {
            audit.reject_applied(reason);
            self.checker_status = lint_audit_summary(&audit);
            self.last_harper_audit = Some(audit);
        }
    }

    pub(super) fn optimistic_insert(&mut self, anchor: DocumentSnapshot, transcript: String) {
        let Some(placed) = self.place_raw_transcript(anchor, transcript) else {
            return;
        };

        match self.checking_provider {
            CheckingProvider::Harper => self.check_placed_transcript_locally(placed),
            CheckingProvider::Codex => self.request_placed_transcript_refinement(placed),
        }
    }

    fn place_raw_transcript(
        &mut self,
        anchor: DocumentSnapshot,
        transcript: String,
    ) -> Option<PlacedTranscript> {
        if self.document.revision() != anchor.revision {
            self.set_notice(
                Notice::new(
                    NoticeSource::Safety,
                    UiState::Warning,
                    "Transcript needs placement",
                    "The document changed while you were speaking, so nothing was inserted at the stale cursor.",
                )
                .recovery("The transcript is saved below; move the cursor and choose Insert last."),
            );
            return None;
        }

        let range = anchor.target_range();
        let raw = fit_literal(&anchor, &transcript);
        let revision_before = self.document.revision();
        if self.document.replace(range.clone(), &raw).is_err() {
            self.set_notice(
                Notice::new(
                    NoticeSource::Safety,
                    UiState::Error,
                    "Couldn’t place the transcript",
                    "The target cursor range failed local validation; no text was changed.",
                )
                .recovery("Move the cursor and choose Insert last to recover the transcript."),
            );
            return None;
        }

        Some(PlacedTranscript {
            range,
            raw,
            changed: self.document.revision() != revision_before,
        })
    }

    fn check_placed_transcript_locally(&mut self, placed: PlacedTranscript) {
        let plan = self.prepare_harper_check(&placed);
        let checked = self
            .harper
            .check_focused(&plan.original_context, plan.focus.clone());
        self.finish_harper_check(plan, checked);
    }

    fn prepare_harper_check(&self, placed: &PlacedTranscript) -> HarperCheckPlan {
        let inserted = placed.range.start..placed.range.start + placed.raw.len();
        let checked_document = self.document.snapshot();
        let context_range = harper_context_range(&checked_document.text, &inserted);
        let focus_start = checked_document.text[context_range.start..inserted.start]
            .chars()
            .count();
        let focus_end = checked_document.text[context_range.start..inserted.end]
            .chars()
            .count();

        HarperCheckPlan {
            original_context: checked_document.text[context_range.clone()].to_owned(),
            context_range,
            focus: focus_start..focus_end,
        }
    }

    fn finish_harper_check(&mut self, plan: HarperCheckPlan, checked: CheckResult) {
        let CheckResult {
            text,
            audit,
            focus_end,
        } = checked;
        self.last_harper_audit = Some(audit);
        self.checker_review = None;
        self.checker_review_open = false;

        let applied = if text == plan.original_context {
            self.report_unchanged_harper_check();
            true
        } else {
            self.apply_harper_correction(plan.context_range.clone(), &text, focus_end)
        };

        if applied {
            let reviewed_range = plan.context_range.start..plan.context_range.start + text.len();
            self.capture_checker_review(reviewed_range, Vec::new(), Vec::new(), Vec::new());
        }
    }

    fn report_unchanged_harper_check(&mut self) {
        self.set_transient_notice(self.default_notice());
    }

    fn apply_harper_correction(
        &mut self,
        context_range: Range<usize>,
        corrected: &str,
        focus_end: usize,
    ) -> bool {
        let Some(relative_cursor) = char_offset_to_byte(corrected, focus_end) else {
            self.reject_harper_application(
                "The corrected cursor position was not a valid UTF-8 boundary.".to_owned(),
            );
            return false;
        };
        let cursor = context_range.start + relative_cursor;

        match self
            .document
            .amend_last_replace_with_cursor(context_range, corrected, cursor)
        {
            Ok(()) => {
                self.set_transient_notice(self.default_notice());
                true
            }
            Err(error) => {
                self.reject_harper_application(format!(
                    "The trusted replacement failed validation: {error:?}."
                ));
                false
            }
        }
    }

    fn capture_checker_review(
        &mut self,
        context_range: Range<usize>,
        manually_applied: Vec<crate::checker::LintRecord>,
        ignored_lints: Vec<crate::checker::LintRecord>,
        ignored_kinds: Vec<harper_core::linting::LintKind>,
    ) {
        let snapshot = self.document.snapshot();
        let Some(context_text) = snapshot.text.get(context_range.clone()).map(str::to_owned) else {
            self.checker_review = None;
            self.refresh_checker_status();
            return;
        };
        let lints = self.harper.review(&context_text);
        let audit_ignored = self
            .last_harper_audit
            .as_ref()
            .map(|audit| audit.ignored.as_slice())
            .unwrap_or_default();
        let mut review_lints = Vec::new();
        let mut ignored = Vec::new();
        for lint in lints {
            if ignored_kinds.contains(&lint.kind) {
                ignored.push(CheckerIgnoredLint {
                    lint,
                    scope: CheckerIgnoreScope::Kind,
                });
            } else if ignored_lints.contains(&lint) {
                ignored.push(CheckerIgnoredLint {
                    lint,
                    scope: CheckerIgnoreScope::Lint,
                });
            } else {
                let reason = audit_ignored
                    .iter()
                    .find(|ignored| {
                        ignored.lint.span == lint.span
                            && ignored.lint.kind == lint.kind
                            && ignored.lint.message == lint.message
                    })
                    .map(|ignored| ignored.reason);
                review_lints.push(CheckerReviewLint { lint, reason });
            }
        }
        let auto_applied = self
            .last_harper_audit
            .as_ref()
            .map(|audit| audit.applied.clone())
            .unwrap_or_default();

        self.checker_status = checker_review_summary(
            auto_applied.len() + manually_applied.len(),
            review_lints.len(),
            ignored.len(),
        );
        self.checker_review = Some(CheckerReview {
            buffer_generation: self.buffer_generation,
            revision: snapshot.revision,
            context_range,
            context_text,
            auto_applied,
            manually_applied,
            ignored_lints,
            ignored_kinds,
            ignored,
            lints: review_lints,
        });
    }

    pub(super) fn open_checker_review(&mut self) -> Task<Message> {
        if self.checking_provider == CheckingProvider::Harper
            && self.checker_review.is_some()
            && self.settings.is_none()
            && self.discard_action.is_none()
        {
            self.checker_review_open = true;
        }
        Task::none()
    }

    pub(super) fn close_checker_review(&mut self) -> Task<Message> {
        self.checker_review_open = false;
        operation::focus(super::EDITOR_ID)
    }

    pub(super) fn apply_checker_suggestion(
        &mut self,
        lint_index: usize,
        suggestion_index: usize,
    ) -> Task<Message> {
        let Some(review) = self.checker_review.as_ref() else {
            return Task::none();
        };
        let Some(review_lint) = review.lints.get(lint_index) else {
            return Task::none();
        };
        let Some(suggestion) = review_lint.lint.suggestions.get(suggestion_index) else {
            return Task::none();
        };

        let context_range = review.context_range.clone();
        let context_text = review.context_text.clone();
        let expected_generation = review.buffer_generation;
        let expected_revision = review.revision;
        let applied_lint = review_lint.lint.clone();
        let (relative_chars, replacement) = suggestion.edit(&applied_lint.span);

        let review_is_current = self.checker_review_is_current(
            expected_generation,
            expected_revision,
            &context_range,
            &context_text,
        );
        let Some(relative_start) = char_offset_to_byte(&context_text, relative_chars.start) else {
            return self.reject_checker_review_application();
        };
        let Some(relative_end) = char_offset_to_byte(&context_text, relative_chars.end) else {
            return self.reject_checker_review_application();
        };
        if !review_is_current || relative_start > relative_end {
            return self.reject_checker_review_application();
        }

        let replace_range =
            context_range.start + relative_start..context_range.start + relative_end;
        if self
            .document
            .replace(replace_range.clone(), &replacement)
            .is_err()
        {
            return self.reject_checker_review_application();
        }

        let mut manually_applied = review.manually_applied.clone();
        manually_applied.push(applied_lint);
        let ignored_lints = remap_ignored_lints_after_edit(
            review.ignored_lints.clone(),
            relative_chars,
            replacement.chars().count(),
        );
        let ignored_kinds = review.ignored_kinds.clone();
        let replaced_len = replace_range.end - replace_range.start;
        let new_context_end = if replacement.len() >= replaced_len {
            context_range.end + (replacement.len() - replaced_len)
        } else {
            context_range.end - (replaced_len - replacement.len())
        };
        self.capture_checker_review(
            context_range.start..new_context_end,
            manually_applied,
            ignored_lints,
            ignored_kinds,
        );
        Task::none()
    }

    pub(super) fn ignore_checker_lint(&mut self, lint_index: usize) -> Task<Message> {
        let Some(review) = self.checker_review.as_ref() else {
            return Task::none();
        };
        let Some(lint) = review.lints.get(lint_index).map(|lint| lint.lint.clone()) else {
            return Task::none();
        };
        if !self.checker_review_is_current(
            review.buffer_generation,
            review.revision,
            &review.context_range,
            &review.context_text,
        ) {
            return self.reject_checker_review_application();
        }

        let mut ignored_lints = review.ignored_lints.clone();
        if !ignored_lints.contains(&lint) {
            ignored_lints.push(lint);
        }
        self.capture_checker_review(
            review.context_range.clone(),
            review.manually_applied.clone(),
            ignored_lints,
            review.ignored_kinds.clone(),
        );
        Task::none()
    }

    pub(super) fn ignore_checker_kind(&mut self, lint_index: usize) -> Task<Message> {
        let Some(review) = self.checker_review.as_ref() else {
            return Task::none();
        };
        let Some(kind) = review.lints.get(lint_index).map(|lint| lint.lint.kind) else {
            return Task::none();
        };
        if !self.checker_review_is_current(
            review.buffer_generation,
            review.revision,
            &review.context_range,
            &review.context_text,
        ) {
            return self.reject_checker_review_application();
        }

        let mut ignored_kinds = review.ignored_kinds.clone();
        if !ignored_kinds.contains(&kind) {
            ignored_kinds.push(kind);
        }
        self.capture_checker_review(
            review.context_range.clone(),
            review.manually_applied.clone(),
            review.ignored_lints.clone(),
            ignored_kinds,
        );
        Task::none()
    }

    fn checker_review_is_current(
        &self,
        expected_generation: u64,
        expected_revision: u64,
        context_range: &Range<usize>,
        context_text: &str,
    ) -> bool {
        self.buffer_generation == expected_generation
            && self.document.revision() == expected_revision
            && self.document.text().get(context_range.clone()) == Some(context_text)
    }

    fn reject_checker_review_application(&mut self) -> Task<Message> {
        self.checker_review_open = false;
        self.checker_review = None;
        self.checker_status = "Review expired · dictate again to refresh.".into();
        self.set_notice(
            Notice::new(
                NoticeSource::Safety,
                UiState::Warning,
                "Checker review expired",
                "The reviewed text changed, so the stored lint no longer identifies an exact action.",
            )
            .recovery("No checker action was applied. Dictate again to refresh the review."),
        );
        operation::focus(super::EDITOR_ID)
    }

    fn reject_harper_application(&mut self, detail: String) {
        self.reject_latest_harper_audit(IgnoreReason::ApplicationFailed);
        self.set_notice(
            Notice::new(
                NoticeSource::Safety,
                UiState::Error,
                "Local grammar correction was skipped",
                detail,
            )
            .recovery("The raw transcript remains in the document; Harper did not roll it back."),
        );
    }

    fn request_placed_transcript_refinement(&mut self, placed: PlacedTranscript) {
        let inserted = placed.range.start..placed.range.start + placed.raw.len();
        let mut refinement = self.document.snapshot();
        refinement.cursor = inserted.end;
        refinement.selection = Some(inserted);
        self.set_notice(Notice::new(
            NoticeSource::Codex,
            UiState::Working,
            "Transcript inserted; requesting bounded refinement",
            "The raw words are already in the local buffer. Codex may only propose a replacement for that captured span.",
        ));
        self.submit_codex(refinement, placed.raw, EditIntent::Insert, placed.changed);
    }

    pub(super) fn submit_codex(
        &mut self,
        snapshot: DocumentSnapshot,
        transcript: String,
        intent: EditIntent,
        amend_optimistic_insert: bool,
    ) {
        let Some(submission) =
            self.prepare_codex_submission(snapshot, transcript, intent, amend_optimistic_insert)
        else {
            return;
        };
        let PreparedCodexSubmission {
            request_id,
            request,
            pending,
        } = submission;

        match self.codex.submit(request) {
            Ok(()) => self.register_codex_submission(request_id, pending),
            Err(error) => self.report_codex_submit_error(error, intent),
        }
    }

    fn prepare_codex_submission(
        &mut self,
        snapshot: DocumentSnapshot,
        transcript: String,
        intent: EditIntent,
        amend_optimistic_insert: bool,
    ) -> Option<PreparedCodexSubmission> {
        let editable_context = match editable_context_range(&snapshot) {
            Ok(range) => range,
            Err(error) => {
                self.codex_status = format!("Codex: {error}");
                self.codex_preview.clear();
                self.set_notice(
                    Notice::new(
                        NoticeSource::Safety,
                        UiState::Error,
                        "Voice edit context was rejected locally",
                        error.to_string(),
                    )
                    .recovery(match intent {
                        EditIntent::Insert => {
                            "Codex made no change and did not roll back the inserted local text."
                        }
                        EditIntent::Command => {
                            "No command edit was applied; the document is unchanged."
                        }
                    }),
                );
                return None;
            }
        };
        let request_id = self.allocate_id();
        let file_name = self
            .file
            .as_deref()
            .and_then(Path::file_name)
            .and_then(ffi::OsStr::to_str)
            .map(str::to_owned);

        Some(PreparedCodexSubmission {
            request_id,
            request: CodexRequest {
                id: request_id,
                snapshot: snapshot.clone(),
                transcript,
                intent,
                file_name,
            },
            pending: PendingEdit {
                buffer_generation: self.buffer_generation,
                editable_context,
                snapshot,
                intent,
                amend_optimistic_insert,
            },
        })
    }

    fn register_codex_submission(&mut self, request_id: u64, pending: PendingEdit) {
        let intent = pending.intent;
        self.pending.insert(request_id, pending);
        self.codex_preview.clear();
        self.codex_state = UiState::Working;
        self.codex_status = "Codex: queued…".into();
        self.set_notice(Notice::new(
            NoticeSource::Codex,
            UiState::Working,
            match intent {
                EditIntent::Insert => "Refining the inserted transcript",
                EditIntent::Command => "Planning a contextual edit",
            },
            match intent {
                EditIntent::Insert => {
                    "The raw words are already in the local buffer and are not rolled back if refinement fails."
                }
                EditIntent::Command => {
                    "No text changes until the returned target passes local safety checks."
                }
            },
        ));
    }

    fn report_codex_submit_error(&mut self, error: CodexSubmitError, intent: EditIntent) {
        let (codex_state, notice_state, title) = match error {
            CodexSubmitError::QueueFull => (UiState::Working, UiState::Warning, "Codex is busy"),
            CodexSubmitError::WorkerStopped => (
                UiState::Error,
                UiState::Error,
                match intent {
                    EditIntent::Insert => "Refinement unavailable",
                    EditIntent::Command => "Command edit not sent",
                },
            ),
        };
        let message = error.to_string();
        self.codex_state = codex_state;
        self.codex_status = format!("Codex: {message}");
        self.set_notice(
            Notice::new(NoticeSource::Codex, notice_state, title, message).recovery(match intent {
                EditIntent::Insert => {
                    "Codex applied no replacement and did not roll back your local edits. Try refinement again later."
                }
                EditIntent::Command => {
                    "Codex applied no command edit. Retry when the service is ready."
                }
            }),
        );
    }

    pub(super) fn handle_codex(&mut self, event: CodexEvent) {
        match event {
            CodexEvent::Starting => self.handle_codex_starting(),
            CodexEvent::Models(models) => self.codex_models = models,
            CodexEvent::Ready { plan, model } => self.handle_codex_ready(plan, model),
            CodexEvent::Working { request_id } => self.handle_codex_working(request_id),
            CodexEvent::Delta { request_id, text } => self.handle_codex_delta(request_id, text),
            CodexEvent::Completed {
                request_id,
                proposal,
            } => self.handle_codex_completed(request_id, proposal),
            CodexEvent::Failed {
                request_id,
                message,
            } => self.handle_codex_failed(request_id, message),
            CodexEvent::Stopped => self.handle_codex_stopped(),
        }
    }

    fn handle_codex_starting(&mut self) {
        self.codex_state = UiState::Working;
        self.codex_status = "Codex: connecting to the signed-in app-server…".into();
    }

    fn handle_codex_ready(&mut self, plan: String, model: String) {
        if self.pending.is_empty() {
            self.codex_state = UiState::Ready;
            self.codex_status = format!("Codex: ChatGPT {plan} · {model}");
        } else {
            self.codex_state = UiState::Working;
            self.codex_status = format!(
                "Codex: {model} · {} edit{} pending",
                self.pending.len(),
                if self.pending.len() == 1 { "" } else { "s" }
            );
        }

        if self.notice.source == NoticeSource::Codex
            && matches!(
                self.notice.state,
                UiState::Warning | UiState::Error | UiState::Offline
            )
        {
            self.set_notice(Notice::new(
                NoticeSource::Codex,
                UiState::Success,
                "Codex is connected again",
                "Voice refinements and contextual commands are available.",
            ));
        }
    }

    fn handle_codex_working(&mut self, request_id: u64) {
        if self.pending_request_is_current(request_id) {
            self.codex_state = UiState::Working;
            self.codex_status = format!("Codex: editing #{request_id}…");
            self.codex_preview.clear();
        }
    }

    fn handle_codex_delta(&mut self, request_id: u64, text: String) {
        if !self.pending_request_is_current(request_id) {
            return;
        }

        self.codex_preview.push_str(&text);
        if self.codex_preview.len() > 180 {
            let start = self.codex_preview.len() - 180;
            let start = next_char_boundary(&self.codex_preview, start);
            self.codex_preview = format!("…{}", &self.codex_preview[start..]);
        }
    }

    fn handle_codex_completed(&mut self, request_id: u64, proposal: ProposedEdit) {
        let Some(pending) = self.pending.get(&request_id) else {
            return;
        };
        if pending.buffer_generation != self.buffer_generation {
            self.pending.remove(&request_id);
            self.codex_preview.clear();
            self.settle_codex_activity();
            return;
        }

        if self.active_utterance.is_some() {
            self.defer_codex_edit_until_dictation_ends(request_id, proposal);
        } else {
            self.apply_codex_edit(request_id, proposal);
        }
    }

    fn defer_codex_edit_until_dictation_ends(&mut self, request_id: u64, proposal: ProposedEdit) {
        self.deferred_codex.push((request_id, proposal));
        self.codex_state = UiState::Working;
        self.codex_status = "Codex: edit ready; applying after dictation".into();
        self.codex_preview.clear();
        self.set_transient_notice(Notice::new(
            NoticeSource::Codex,
            UiState::Working,
            "Refinement ready and safely deferred",
            "It will be validated and applied after the active dictation ends.",
        ));
    }

    fn handle_codex_failed(&mut self, request_id: Option<u64>, message: String) {
        let scope = self.codex_failure_scope(request_id);
        self.codex_state = UiState::Error;
        self.codex_status = format!("Codex: {message}");
        self.codex_preview.clear();
        self.set_notice(Self::codex_failure_notice(scope, message));
    }

    fn codex_failure_scope(&mut self, request_id: Option<u64>) -> CodexFailureScope {
        let Some(request_id) = request_id else {
            return CodexFailureScope::Service;
        };
        let Some(pending) = self.pending.remove(&request_id) else {
            return CodexFailureScope::Background;
        };
        if pending.buffer_generation != self.buffer_generation {
            CodexFailureScope::Background
        } else {
            CodexFailureScope::Current(pending.intent)
        }
    }

    fn codex_failure_notice(scope: CodexFailureScope, message: String) -> Notice {
        match scope {
            CodexFailureScope::Service => Notice::new(
                NoticeSource::Codex,
                UiState::Error,
                "Codex is unavailable",
                message,
            )
            .recovery("Raw dictation and typed editing still work. Check the Codex CLI, `codex login status`, and connectivity."),
            CodexFailureScope::Background => Notice::new(
                NoticeSource::Codex,
                UiState::Error,
                "Codex background request failed",
                message,
            )
            .recovery(
                "No Codex change was applied to the current document. Local editing remains available.",
            ),
            CodexFailureScope::Current(intent) => Notice::new(
                NoticeSource::Codex,
                UiState::Error,
                match intent {
                    EditIntent::Insert => "Couldn’t refine this dictation",
                    EditIntent::Command => "Contextual edit failed",
                },
                message,
            )
            .recovery(match intent {
                EditIntent::Insert => {
                    "Codex applied no replacement and did not roll back your local edits. Retry when the service is ready."
                }
                EditIntent::Command => {
                    "Codex applied no command edit. Retry when the service is ready."
                }
            }),
        }
    }

    fn handle_codex_stopped(&mut self) {
        let preserve_failure = self.codex_state == UiState::Error;
        let had_pending = !self.pending.is_empty();
        self.pending.clear();
        self.deferred_codex.clear();
        self.codex_preview.clear();
        self.codex_state = UiState::Offline;

        if !preserve_failure {
            self.codex_status = "Codex: stopped".into();
            if had_pending {
                self.set_notice(
                    Notice::new(
                        NoticeSource::Codex,
                        UiState::Warning,
                        "Codex stopped before finishing",
                        "Pending Codex edits or refinements were not applied; Codex did not roll back local text.",
                    )
                    .recovery("Restart Talkdown after checking `codex login status`."),
                );
            }
        }
    }

    fn pending_request_is_current(&self, request_id: u64) -> bool {
        self.pending
            .get(&request_id)
            .is_some_and(|pending| pending.buffer_generation == self.buffer_generation)
    }

    fn apply_codex_edit(&mut self, request_id: u64, proposal: ProposedEdit) {
        let Some(pending) = self.pending.remove(&request_id) else {
            return;
        };

        let outcome = self.validate_and_apply_codex_edit(&pending, &proposal);
        self.finish_codex_edit(outcome);
    }

    fn validate_and_apply_codex_edit(
        &mut self,
        pending: &PendingEdit,
        proposal: &ProposedEdit,
    ) -> CodexEditOutcome {
        if pending.buffer_generation != self.buffer_generation {
            return CodexEditOutcome::Rejected(CodexEditRejection::PreviousDocument);
        }

        let resolved = match Self::resolve_codex_proposal(pending, proposal) {
            Ok(resolved) => resolved,
            Err(rejection) => return CodexEditOutcome::Rejected(rejection),
        };

        match self.apply_resolved_codex_proposal(pending, proposal, resolved) {
            Ok(summary) => {
                let summary = compact_copy(&summary, 80);
                let summary = if summary.is_empty() {
                    "Voice edit applied".to_owned()
                } else {
                    summary
                };
                CodexEditOutcome::Applied { summary }
            }
            Err(rejection) => CodexEditOutcome::Rejected(rejection),
        }
    }

    fn resolve_codex_proposal(
        pending: &PendingEdit,
        proposal: &ProposedEdit,
    ) -> Result<ResolvedCodexProposal, CodexEditRejection> {
        if pending.intent == EditIntent::Insert {
            let expected = pending
                .snapshot
                .selection
                .as_ref()
                .and_then(|range| pending.snapshot.text.get(range.clone()));
            if proposal.anchor != Anchor::Selection || expected != Some(proposal.target.as_str()) {
                return Err(CodexEditRejection::UnsafeRefinement);
            }
        }

        let original = resolve(&pending.snapshot, proposal)
            .map_err(|_| CodexEditRejection::UnresolvedTarget)?;
        if original.range.start < pending.editable_context.start
            || original.range.end > pending.editable_context.end
        {
            return Err(CodexEditRejection::OutsideSharedContext);
        }

        Ok(ResolvedCodexProposal {
            range: original.range,
            replacement: original.replacement,
            summary: original.summary,
        })
    }

    fn apply_resolved_codex_proposal(
        &mut self,
        pending: &PendingEdit,
        proposal: &ProposedEdit,
        resolved: ResolvedCodexProposal,
    ) -> Result<String, CodexEditRejection> {
        let current = self.document.snapshot();
        if current.revision == pending.snapshot.revision {
            let result = if pending.amend_optimistic_insert {
                self.document
                    .amend_last_replace(resolved.range, &resolved.replacement)
            } else {
                self.document.replace(resolved.range, &resolved.replacement)
            };
            return result
                .map(|()| resolved.summary)
                .map_err(|_| CodexEditRejection::DocumentRejected);
        }

        if proposal.target.is_empty() {
            return Err(CodexEditRejection::StaleInsertion);
        }

        let rebased =
            rebase_exact(&current, proposal).map_err(|_| CodexEditRejection::MissingStaleTarget)?;
        if !rebased.is_unambiguous() {
            return Err(CodexEditRejection::AmbiguousStaleTarget);
        }

        self.document
            .replace(rebased.range, &rebased.replacement)
            .map(|()| resolved.summary)
            .map_err(|_| CodexEditRejection::DocumentRejected)
    }

    fn finish_codex_edit(&mut self, outcome: CodexEditOutcome) {
        match outcome {
            CodexEditOutcome::Applied { summary } => {
                self.set_notice(Notice::new(
                    NoticeSource::Codex,
                    UiState::Success,
                    summary,
                    "The edit passed local target validation. One Undo restores the previous text.",
                ));
                self.settle_codex_activity();
                self.codex_preview.clear();
            }
            CodexEditOutcome::Rejected(rejection) => {
                let notice = rejection.notice();
                self.reject_codex_edit(notice.title, notice.detail, notice.recovery);
            }
        }
    }

    fn reject_codex_edit(&mut self, title: &str, detail: &str, recovery: &str) {
        self.settle_codex_activity();
        if self.codex_state == UiState::Ready {
            self.codex_status = "Codex: ready · suggestion rejected locally".into();
        }
        self.codex_preview.clear();
        self.set_notice(
            Notice::new(NoticeSource::Safety, UiState::Warning, title, detail).recovery(recovery),
        );
    }

    pub(super) fn apply_deferred_codex(&mut self) {
        for (request_id, proposal) in std::mem::take(&mut self.deferred_codex) {
            self.apply_codex_edit(request_id, proposal);
        }
    }

    fn settle_codex_activity(&mut self) {
        if self.pending.is_empty() {
            self.codex_state = UiState::Ready;
            self.codex_status = "Codex: ready".into();
        } else {
            self.codex_state = UiState::Working;
            self.codex_status = format!(
                "Codex: {} edit{} still pending…",
                self.pending.len(),
                if self.pending.len() == 1 { "" } else { "s" }
            );
        }
    }
}

fn checker_review_summary(applied: usize, remaining: usize, ignored: usize) -> String {
    format!(
        "Latest check · {applied} applied · {remaining} to review · {ignored} ignored. Click to inspect."
    )
}

fn remap_ignored_lints_after_edit(
    ignored: Vec<crate::checker::LintRecord>,
    edited: Range<usize>,
    replacement_len: usize,
) -> Vec<crate::checker::LintRecord> {
    let replaced_len = edited.end - edited.start;
    let delta = replacement_len as isize - replaced_len as isize;

    ignored
        .into_iter()
        .filter_map(|mut lint| {
            if lint.span.end <= edited.start {
                Some(lint)
            } else if edited.end <= lint.span.start {
                lint.span.start = lint.span.start.checked_add_signed(delta)?;
                lint.span.end = lint.span.end.checked_add_signed(delta)?;
                Some(lint)
            } else {
                None
            }
        })
        .collect()
}
