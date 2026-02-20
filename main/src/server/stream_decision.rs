use super::StreamRuntimeOptions;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum OutputDisposition {
    SkipDuplicateMessageStart,
    SkipPrematureMessageStop,
    Accepted {
        message_stop: bool,
        business_event: bool,
    },
}

#[derive(Debug, Default, Clone)]
pub(crate) struct StreamDecisionState {
    pub saw_response_completed: bool,
    pub saw_response_failed: bool,
    pub saw_sibling_tool_call_error: bool,
    pub saw_message_stop: bool,
    pub sent_message_start_to_client: bool,
    pub emitted_non_heartbeat_event: bool,
    pub emitted_business_event: bool,
    pub stall_retry_attempts: u32,
    pub stall_detected_logged: bool,
    pub stall_retry_skipped_logged: bool,
    pub empty_completion_retry_attempts: u32,
    pub empty_completion_retry_succeeded: bool,
    pub incomplete_stream_retry_attempts: u32,
    pub incomplete_stream_retry_succeeded: bool,
    pub sibling_tool_error_retry_attempted: bool,
    pub emitted_empty_completion_fallback_notice: bool,
    pub fallback_completion_injected: bool,
    pub logged_premature_stop_suppression: bool,
    pub stream_close_cause: Option<&'static str>,
}

impl StreamDecisionState {
    pub fn classify_output(&mut self, output: &str, is_codex_stream: bool) -> OutputDisposition {
        if super::should_drop_duplicate_message_start(
            output,
            &mut self.sent_message_start_to_client,
        ) {
            return OutputDisposition::SkipDuplicateMessageStart;
        }

        if super::should_suppress_premature_message_stop(
            output,
            is_codex_stream,
            self.saw_response_completed,
            self.saw_response_failed,
        ) {
            return OutputDisposition::SkipPrematureMessageStop;
        }

        let message_stop = super::chunk_is_message_stop(output);
        if message_stop {
            self.saw_message_stop = true;
        }

        self.emitted_non_heartbeat_event = true;
        let business_event = super::is_business_stream_output(output);
        if business_event {
            self.emitted_business_event = true;
        }

        OutputDisposition::Accepted {
            message_stop,
            business_event,
        }
    }

    pub fn on_retry_success_reset(&mut self) {
        self.saw_response_completed = false;
        self.saw_response_failed = false;
        self.saw_sibling_tool_call_error = false;
        self.saw_message_stop = false;
        self.stall_detected_logged = false;
        self.stall_retry_skipped_logged = false;
    }

    pub fn allow_stall_retry(&self, opts: StreamRuntimeOptions) -> bool {
        let phase_retry_allowed = if opts.stall_retry_only_heartbeat_phase {
            !self.emitted_non_heartbeat_event
        } else {
            !self.emitted_business_event
        };

        opts.enable_stall_retry
            && opts.stall_retry_max_attempts > 0
            && self.stall_retry_attempts < opts.stall_retry_max_attempts
            && phase_retry_allowed
    }

    pub fn stall_retry_skip_reason(&self, opts: StreamRuntimeOptions) -> &'static str {
        if !opts.enable_stall_retry {
            "disabled"
        } else if opts.stall_retry_max_attempts == 0 {
            "max_attempts_zero"
        } else if self.stall_retry_attempts >= opts.stall_retry_max_attempts {
            "attempts_exhausted"
        } else if opts.stall_retry_only_heartbeat_phase && self.emitted_non_heartbeat_event {
            "non_heartbeat_event_emitted"
        } else if self.emitted_business_event {
            "business_event_emitted"
        } else {
            "guard_blocked"
        }
    }

    pub fn allow_sibling_tool_retry(&self, has_serial_fallback: bool) -> bool {
        self.saw_response_failed
            && self.saw_sibling_tool_call_error
            && !self.saw_message_stop
            && !self.emitted_business_event
            && !self.sibling_tool_error_retry_attempted
            && has_serial_fallback
    }

    pub fn allow_incomplete_retry(&self, opts: StreamRuntimeOptions) -> bool {
        let stream_incomplete = !self.saw_response_completed;
        stream_incomplete
            && !self.saw_response_failed
            && !self.saw_message_stop
            && opts.enable_incomplete_stream_retry
            && opts.incomplete_stream_retry_max_attempts > 0
            && self.incomplete_stream_retry_attempts < opts.incomplete_stream_retry_max_attempts
    }

    pub fn incomplete_retry_skip_reason(&self, opts: StreamRuntimeOptions) -> &'static str {
        if !opts.enable_incomplete_stream_retry {
            "disabled"
        } else if opts.incomplete_stream_retry_max_attempts == 0 {
            "max_attempts_zero"
        } else if self.incomplete_stream_retry_attempts >= opts.incomplete_stream_retry_max_attempts
        {
            "attempts_exhausted"
        } else if self.saw_response_failed {
            "response_failed_seen"
        } else {
            "guard_blocked"
        }
    }

    pub fn allow_empty_completion_retry(&self, opts: StreamRuntimeOptions) -> bool {
        self.saw_response_completed
            && !self.saw_response_failed
            && !self.emitted_business_event
            && !self.saw_message_stop
            && opts.enable_empty_completion_retry
            && opts.empty_completion_retry_max_attempts > 0
            && self.empty_completion_retry_attempts < opts.empty_completion_retry_max_attempts
    }

    pub fn empty_completion_retry_skip_reason(&self, opts: StreamRuntimeOptions) -> &'static str {
        if !opts.enable_empty_completion_retry {
            "disabled"
        } else if opts.empty_completion_retry_max_attempts == 0 {
            "max_attempts_zero"
        } else if self.empty_completion_retry_attempts >= opts.empty_completion_retry_max_attempts {
            "attempts_exhausted"
        } else {
            "guard_blocked"
        }
    }

    pub fn should_emit_empty_notice(&self) -> bool {
        self.saw_response_completed
            && !self.saw_response_failed
            && !self.emitted_business_event
            && !self.saw_message_stop
            && !self.empty_completion_retry_succeeded
    }

    pub fn stream_outcome(&self) -> &'static str {
        if !self.saw_response_completed {
            if self.saw_response_failed {
                "failed"
            } else {
                "incomplete"
            }
        } else if !self.emitted_business_event {
            "empty_completed"
        } else {
            "success"
        }
    }
}

#[cfg(test)]
mod tests {
    use super::super::StreamRuntimeOptions;
    use super::{OutputDisposition, StreamDecisionState};

    fn opts() -> StreamRuntimeOptions {
        StreamRuntimeOptions {
            force_stream_for_codex: true,
            enable_sse_frame_parser: true,
            enable_stream_heartbeat: true,
            stream_heartbeat_interval_ms: 3_000,
            enable_stream_log_sampling: true,
            stream_log_sample_every_n: 20,
            stream_log_max_chars: 512,
            enable_stream_metrics: true,
            enable_stream_event_metrics: true,
            stream_silence_warn_ms: 20_000,
            stream_silence_error_ms: 90_000,
            enable_stall_retry: true,
            stall_timeout_ms: 60_000,
            stall_retry_max_attempts: 2,
            stall_retry_only_heartbeat_phase: false,
            enable_empty_completion_retry: true,
            empty_completion_retry_max_attempts: 1,
            enable_incomplete_stream_retry: true,
            incomplete_stream_retry_max_attempts: 1,
        }
    }

    #[test]
    fn classify_output_suppresses_premature_stop_for_codex() {
        let mut state = StreamDecisionState::default();
        let output = "event: message_stop\ndata: {\"type\":\"message_stop\"}\n\n";
        let decision = state.classify_output(output, true);
        assert!(matches!(
            decision,
            OutputDisposition::SkipPrematureMessageStop
        ));
        assert!(!state.saw_message_stop);
    }

    #[test]
    fn classify_output_accepts_message_stop_after_completion() {
        let mut state = StreamDecisionState {
            saw_response_completed: true,
            ..Default::default()
        };
        let output = "event: message_stop\ndata: {\"type\":\"message_stop\"}\n\n";
        let decision = state.classify_output(output, true);
        assert!(matches!(
            decision,
            OutputDisposition::Accepted {
                message_stop: true,
                ..
            }
        ));
        assert!(state.saw_message_stop);
    }

    #[test]
    fn guard_methods_reflect_retry_policies() {
        let mut state = StreamDecisionState::default();
        assert!(state.allow_stall_retry(opts()));
        assert!(state.allow_incomplete_retry(opts()));
        assert!(!state.allow_empty_completion_retry(opts()));

        state.saw_response_completed = true;
        assert!(state.allow_empty_completion_retry(opts()));

        state.incomplete_stream_retry_attempts = 1;
        assert!(!state.allow_incomplete_retry(opts()));
    }

    #[test]
    fn sibling_tool_retry_guard_requires_clean_failed_state() {
        let mut state = StreamDecisionState {
            saw_response_failed: true,
            saw_sibling_tool_call_error: true,
            ..Default::default()
        };
        assert!(state.allow_sibling_tool_retry(true));

        state.emitted_business_event = true;
        assert!(!state.allow_sibling_tool_retry(true));

        state.emitted_business_event = false;
        state.sibling_tool_error_retry_attempted = true;
        assert!(!state.allow_sibling_tool_retry(true));

        state.sibling_tool_error_retry_attempted = false;
        assert!(!state.allow_sibling_tool_retry(false));
    }

    #[test]
    fn empty_notice_and_stream_outcome_follow_state() {
        let mut state = StreamDecisionState::default();
        assert_eq!(state.stream_outcome(), "incomplete");
        assert!(!state.should_emit_empty_notice());

        state.saw_response_failed = true;
        assert_eq!(state.stream_outcome(), "failed");

        state = StreamDecisionState {
            saw_response_completed: true,
            ..Default::default()
        };
        assert_eq!(state.stream_outcome(), "empty_completed");
        assert!(state.should_emit_empty_notice());

        state.emitted_business_event = true;
        assert_eq!(state.stream_outcome(), "success");
        assert!(!state.should_emit_empty_notice());
    }

    #[test]
    fn retry_success_reset_clears_transient_flags() {
        let mut state = StreamDecisionState {
            saw_response_completed: true,
            saw_response_failed: true,
            saw_sibling_tool_call_error: true,
            saw_message_stop: true,
            stall_detected_logged: true,
            stall_retry_skipped_logged: true,
            ..Default::default()
        };

        state.on_retry_success_reset();

        assert!(!state.saw_response_completed);
        assert!(!state.saw_response_failed);
        assert!(!state.saw_sibling_tool_call_error);
        assert!(!state.saw_message_stop);
        assert!(!state.stall_detected_logged);
        assert!(!state.stall_retry_skipped_logged);
    }
}
