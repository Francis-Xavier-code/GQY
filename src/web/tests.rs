use super::*;

    use super::*;
    use crate::question::{QuestionOption, QuestionPrompt};

    #[test]
    fn assistant_sentinels_are_never_exposed() {
        assert_eq!(
            redact_internal_assistant_text(crate::state::pending_placeholder()),
            ""
        );
        assert_eq!(
            redact_internal_assistant_text(crate::state::interrupted_text()),
            ""
        );
        let combined = format!("before {} after", crate::state::interrupted_text());
        let redacted = redact_internal_assistant_text(&combined);
        assert_eq!(redacted, "before  after");
        assert!(!redacted.contains("system-reminder"));
    }

    #[test]
    fn persisted_meme_assets_hide_their_descriptive_caption() {
        let asset = ImageAsset {
            asset_id: "img_test".to_string(),
            turn_id: "turn_test".to_string(),
            tool_id: Some("tool_test".to_string()),
            mime: "image/png".to_string(),
            width: 64,
            height: 64,
            alt: "猫猫 开心 & <得意>".to_string(),
            created_at: "2026-01-01T00:00:00Z".to_string(),
        };
        let reports = vec![
            "<sent_meme>发送了一个表情包：id=sha256:test；description=猫猫 开心 &amp; &lt;得意&gt;</sent_meme>"
                .to_string(),
        ];

        assert!(meme_asset_caption_hidden(&asset, &reports));
        assert!(!meme_asset_caption_hidden(
            &asset,
            &["normal tool output".to_string()]
        ));
    }

    #[test]
    fn cookie_parser_matches_an_exact_cookie_name() {
        let mut headers = HeaderMap::new();
        headers.insert(
            COOKIE,
            HeaderValue::from_static("other=1; gqy_session=secret-token; suffix=2"),
        );
        assert_eq!(cookie_value(&headers, AUTH_COOKIE), Some("secret-token"));
        assert_eq!(cookie_value(&headers, "session"), None);
    }

    #[test]
    fn lock_mutex_recovers_from_poisoned_guard() {
        let mutex = Mutex::new(7u32);
        let _ = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            let _guard = lock_mutex(&mutex);
            panic!("poison the mutex on purpose");
        }));
        assert!(mutex.is_poisoned());
        assert_eq!(*lock_mutex(&mutex), 7);
    }

    #[test]
    fn mutation_guard_requires_matching_origin_after_auth() {
        // require_mutation = require_auth + origin_is_allowed
        // 无密码时 auth 恒通过；跨站 Origin 仍必须被拒绝（CSRF 防护）
        let mut headers = HeaderMap::new();
        headers.insert(HOST, HeaderValue::from_static("127.0.0.1:4096"));
        headers.insert(ORIGIN, HeaderValue::from_static("http://evil.example"));
        assert!(!origin_is_allowed(&headers));
        headers.insert(ORIGIN, HeaderValue::from_static("http://127.0.0.1:4096"));
        assert!(origin_is_allowed(&headers));
    }

    #[test]
    fn origin_check_accepts_absent_or_current_host_origin() {
        let mut headers = HeaderMap::new();
        assert!(origin_is_allowed(&headers));
        headers.insert(HOST, HeaderValue::from_static("192.168.1.20:4096"));
        headers.insert(ORIGIN, HeaderValue::from_static("http://127.0.0.1:4096"));
        assert!(!origin_is_allowed(&headers));
        headers.insert(ORIGIN, HeaderValue::from_static("http://192.168.1.20:4096"));
        assert!(origin_is_allowed(&headers));
        headers.append(ORIGIN, HeaderValue::from_static("http://192.168.1.20:4096"));
        assert!(!origin_is_allowed(&headers));
    }

    #[test]
    fn optional_password_auth_issues_server_side_sessions_and_limits_failures() {
        let disabled = WebAuth::new(None);
        assert!(disabled.is_authenticated(None));

        let auth = WebAuth::new(Some("correct horse"));
        let peer = IpAddr::V4(Ipv4Addr::LOCALHOST);
        assert!(!auth.is_authenticated(None));
        assert!(matches!(
            auth.login(peer, "wrong"),
            Err(LoginFailure::Invalid)
        ));
        let token = auth.login(peer, "correct horse").unwrap();
        assert!(auth.is_authenticated(Some(&token)));

        let limited = WebAuth::new(Some("secret"));
        for _ in 0..LOGIN_ATTEMPT_LIMIT {
            assert!(matches!(
                limited.login(peer, "wrong"),
                Err(LoginFailure::Invalid)
            ));
        }
        assert!(matches!(
            limited.login(peer, "secret"),
            Err(LoginFailure::RateLimited)
        ));
    }

    #[test]
    fn model_selection_rejects_empty_and_duplicate_pools() {
        assert!(validate_model_selection(Vec::new()).is_err());
        let model = ActiveProviderModelConfig {
            provider_id: "provider".to_string(),
            model: "model".to_string(),
        };
        assert!(validate_model_selection(vec![model.clone()]).is_ok());
        assert!(validate_model_selection(vec![model.clone(), model]).is_err());
    }

    #[test]
    fn config_response_never_serializes_secret_values() {
        let mut config = AppConfig::default();
        config.providers[0].api_key = Some("provider-secret".to_string());
        config.plugins.web.tavily_api_keys = vec!["tavily-secret".to_string()];
        config.plugins.exchange_rate.api_key = "exchange-secret".to_string();
        config.plugins.image_generation.api_keys = vec!["image-secret".to_string()];
        let paths = tempfile::tempdir().unwrap();
        let paths = GqyPaths {
            config_dir: paths.path().join("config"),
            config_file: paths.path().join("config/config.jsonc"),
            skills_dir: paths.path().join("config/skills"),
            data_dir: paths.path().join("data"),
            cache_dir: paths.path().join("cache"),
            state_dir: paths.path().join("state"),
            pictures_dir: paths.path().join("pictures"),
            fish_hook_file: paths.path().join("fish"),
            bash_hook_file: paths.path().join("bash"),
            zsh_hook_file: paths.path().join("zsh"),
            scripts_dir: paths.path().join("scripts"),
            system_scripts_dir: paths.path().join("system-scripts"),
            share_dir: PathBuf::new(),
            kb_dir: PathBuf::new(),
        };
        let response = config_response(
            &config,
            ContextSnapshot {
                tokens: 0,
                window: None,
            },
            &paths,
        )
        .unwrap();
        let serialized = serde_json::to_string(&response).unwrap();
        assert!(!serialized.contains("provider-secret"));
        assert!(!serialized.contains("tavily-secret"));
        assert!(!serialized.contains("exchange-secret"));
        assert!(!serialized.contains("image-secret"));
        assert_eq!(response.secret_states["providers.0.api_key"], true);
        assert_eq!(response.secret_states["plugins.web.tavily_api_keys"], true);
        assert!(response.config.get("memory").is_some());
    }

    #[test]
    fn omitted_provider_secret_does_not_follow_array_position_after_rename() {
        let mut current = AppConfig::default();
        current.providers[0].id = "first".to_string();
        current.providers[0].api_key = Some("first-secret".to_string());
        let mut candidate = current.clone();
        candidate.providers[0].id = "renamed".to_string();
        candidate.providers[0].api_key = None;
        restore_config_secrets(&mut candidate, &current, &HashMap::new()).unwrap();
        assert_eq!(candidate.providers[0].api_key, None);
    }

    #[test]
    fn explicit_secret_clear_removes_a_provider_key() {
        let mut current = AppConfig::default();
        current.providers[0].api_key = Some("secret".to_string());
        let mut candidate = current.clone();
        candidate.providers[0].api_key = None;
        let mutations = HashMap::from([("providers.0.api_key".to_string(), SecretMutation::Clear)]);
        restore_config_secrets(&mut candidate, &current, &mutations).unwrap();
        assert_eq!(candidate.providers[0].api_key, None);
    }

    #[test]
    fn stale_event_cursor_receives_resync_marker() {
        let events = EventHub::new();
        for index in 0..=EVENT_CAPACITY {
            events.publish("test", json!({ "index": index }));
        }
        let replay = events.replay_after(0);
        assert_eq!(replay.len(), 1);
        assert_eq!(replay[0].kind, "resync_required");
        assert_eq!(replay[0].id, events.latest_id());
        let next = events.publish("after-resync", json!({}));
        assert!(next > replay[0].id);
    }

    #[test]
    fn replay_after_cursor_is_ordered_and_exclusive() {
        let events = EventHub::new();
        events.publish("one", json!({}));
        events.publish("two", json!({}));
        events.publish("three", json!({}));
        let replay = events.replay_after(1);
        assert_eq!(
            replay.iter().map(|record| record.id).collect::<Vec<_>>(),
            vec![2, 3]
        );
    }

    #[test]
    fn future_event_cursor_requests_resync_after_server_restart() {
        let events = EventHub::new();
        let replay = events.replay_after(42);
        assert_eq!(replay.len(), 1);
        assert_eq!(replay[0].kind, "resync_required");
    }

    #[test]
    fn answer_validation_trims_values_and_rejects_control_characters() {
        let request = sample_question();
        assert_eq!(
            normalize_answers(&request, vec![vec!["  All  ".to_string()]]).unwrap(),
            vec![vec!["All".to_string()]]
        );
        assert!(normalize_answers(&request, vec![vec!["bad\nanswer".to_string()]]).is_err());
    }

    #[test]
    fn invalid_answer_keeps_question_pending() {
        let broker = QuestionBroker::new();
        let (responder, mut response) = oneshot::channel();
        let question_id = broker.insert("run_test", sample_question(), responder);
        let invalid = broker.answer(&question_id, vec![Vec::new()], |_, _| {
            panic!("invalid answer must not be published")
        });
        assert!(matches!(invalid, Err(AnswerFailure::Invalid(_))));
        assert!(broker.pending.lock().unwrap().contains_key(&question_id));

        broker
            .answer(
                &question_id,
                vec![vec![" All ".to_string()]],
                |run_id, answers| {
                    assert_eq!(run_id, "run_test");
                    assert_eq!(answers, &vec![vec!["All".to_string()]]);
                },
            )
            .unwrap();
        assert!(matches!(
            response.try_recv().unwrap(),
            QuestionResponse::Answered(answers) if answers == vec![vec!["All".to_string()]]
        ));
    }

    #[test]
    fn closed_question_responder_does_not_publish_an_answer() {
        let broker = QuestionBroker::new();
        let (responder, response) = oneshot::channel();
        drop(response);
        let question_id = broker.insert("run_test", sample_question(), responder);
        let mut published = false;
        let result = broker.answer(&question_id, vec![vec!["All".to_string()]], |_, _| {
            published = true
        });
        assert!(matches!(result, Err(AnswerFailure::Gone)));
        assert!(!published);
    }

    fn sample_question() -> QuestionRequest {
        QuestionRequest {
            questions: vec![QuestionPrompt {
                header: "Scope".to_string(),
                question: "Which scope?".to_string(),
                options: vec![QuestionOption {
                    label: "All".to_string(),
                    description: String::new(),
                }],
                multiple: false,
                custom: true,
            }],
        }
    }

    #[test]
    fn content_limit_counts_characters() {
        assert!(validate_content("x".repeat(MAX_CONTENT_CHARS)).is_ok());
        let error = validate_content("界".repeat(MAX_CONTENT_CHARS + 1)).unwrap_err();
        assert_eq!(error.status, StatusCode::PAYLOAD_TOO_LARGE);
    }

