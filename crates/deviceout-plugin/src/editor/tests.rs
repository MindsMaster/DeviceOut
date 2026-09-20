use super::*;

struct DeviceCardHarness {
    ctx: egui::Context,
    wiring: Wiring,
    snapshot: Option<UiState>,
    time: f64,
}

impl DeviceCardHarness {
    fn new(name: &str) -> Self {
        let ctx = egui::Context::default();
        theme::install(&ctx);
        let params = Arc::new(DeviceOutParams::default());
        *params.device_id.write() = "test-device".into();
        let device = DeviceInfo {
            id: "test-device".into(),
            name: name.into(),
            is_default: false,
            mix_format: StreamFormat {
                sample_rate: 48_000,
                channels: 2,
                sample_format: SampleFormat::F32,
            },
        };

        Self {
            ctx,
            wiring: Wiring {
                params,
                engine: Arc::new(EngineController::default()),
                devices: Arc::new(RwLock::new(vec![device])),
            },
            snapshot: None,
            time: 0.0,
        }
    }

    fn frame(
        &mut self,
        width: f32,
        events: Vec<egui::Event>,
    ) -> (egui::Rect, Vec<egui::epaint::TextShape>) {
        self.time += 0.1;
        let input = egui::RawInput {
            screen_rect: Some(egui::Rect::from_min_size(
                egui::Pos2::ZERO,
                egui::vec2(width, 560.0),
            )),
            time: Some(self.time),
            events,
            ..Default::default()
        };
        let mut card_rect = egui::Rect::NOTHING;
        let output = self.ctx.run(input, |ctx| {
            egui::CentralPanel::default()
                .frame(theme::root_frame())
                .show(ctx, |ui| {
                    device_card(ui, &self.wiring, self.snapshot.as_ref());
                    card_rect = ui.min_rect();
                });
        });
        let mut text = Vec::new();
        for clipped in output.shapes {
            collect_text(clipped.shape, &mut text);
        }
        (card_rect, text)
    }
}

fn failed_snapshot() -> UiState {
    UiState {
        state: deviceout_engine::EngineState::Failed,
        error: Some(Fault {
            kind: FaultKind::ChannelMismatch {
                device: 8,
                source: 2,
            },
            detail: "设备 8 声道，DAW 2 声道，不一致".into(),
            attempt: 0,
        }),
        fill_fraction: 0.0,
        capacity_frames: 0,
        drift_ppm: None,
        raw_drift_ppm: 0.0,
        underruns: 0,
        overruns: 0,
        dropout_seconds: 0.0,
        clamp_events: 0,
        sink_rate_hz: 48_000.0,
        period_frames: 0,
        latency_ms: 0.0,
        device_starvations: 0,
        reconnects: 0,
        frames_discarded: 0,
    }
}

#[test]
fn every_fault_kind_uses_the_requested_language() {
    for lang in deviceout_i18n::Lang::ALL {
        let strings = lang.strings();
        let mismatch = fill(
            strings.fault_channel_mismatch,
            &[("device", "8"), ("source", "2")],
        );
        for (kind, expected) in [
            (FaultKind::DeviceNotFound, strings.fault_device_not_found),
            (FaultKind::DeviceLost, strings.fault_device_lost),
            (
                FaultKind::ChannelMismatch {
                    device: 8,
                    source: 2,
                },
                mismatch.as_str(),
            ),
            (
                FaultKind::UnsupportedFormat,
                strings.fault_unsupported_format,
            ),
            (FaultKind::ComInit, strings.fault_audio_system),
            (FaultKind::Enumeration, strings.fault_audio_system),
            (FaultKind::StreamInit, strings.fault_stream_init),
            (FaultKind::Stream, strings.fault_stream),
            (FaultKind::Resample, strings.fault_resample),
            (FaultKind::Config, strings.fault_engine),
            (FaultKind::Thread, strings.fault_engine),
        ] {
            let mut fault = Fault {
                kind,
                detail: "不应显示的内部诊断信息".into(),
                attempt: 0,
            };
            assert_eq!(fault_text(&fault, strings), expected, "{}", lang.tag());
            fault.attempt = 3;
            assert_eq!(
                fault_text(&fault, strings),
                fill(
                    strings.fault_retry,
                    &[("message", expected), ("attempt", "3")]
                ),
                "{}",
                lang.tag()
            );
        }
    }
}

#[test]
fn fault_tooltip_uses_the_localized_message_and_wraps_within_the_window() {
    let mut harness = DeviceCardHarness::new("USB DAC");
    let snapshot = failed_snapshot();
    let fault = snapshot.error.as_ref().unwrap();
    let message = fault_text(fault, t());
    let detail = fault.detail.clone();
    harness.snapshot = Some(snapshot);
    let (card, text) = harness.frame(240.0, Vec::new());
    assert!(card.right() <= 223.0);
    let alert = text
        .iter()
        .find(|text| text.galley.job.text == message)
        .unwrap();
    assert!(alert.galley.rows.len() > 1);
    let pointer = alert.pos + egui::vec2(5.0, 5.0);
    harness.frame(240.0, vec![egui::Event::PointerMoved(pointer)]);
    harness.time += 1.0;
    harness.frame(240.0, Vec::new());
    let (_, text) = harness.frame(240.0, Vec::new());
    let messages: Vec<_> = text
        .iter()
        .filter(|text| text.galley.job.text == message)
        .collect();
    assert_eq!(messages.len(), 2);
    assert!(text.iter().all(|text| text.galley.job.text != detail));
    for message in messages {
        assert!(message.pos.x + message.galley.rect.right() <= 240.0);
    }
}

#[test]
fn update_failure_tooltips_use_localized_guidance_in_every_language() {
    let raw_error = "feed signature rejected: 内部错误详情";
    for lang in deviceout_i18n::Lang::ALL {
        let strings = lang.strings();
        let check_failed = deviceout_update::State {
            last_error: Some(raw_error.into()),
            ..Default::default()
        };
        let (status, color, tip) = update_status_text("1.1.0", None, &check_failed, false, strings);
        assert_eq!(status, strings.check_failed);
        assert_eq!(color, theme::RED);
        assert_eq!(tip.as_deref(), Some(strings.check_failed_tip));
        assert_ne!(tip.as_deref(), Some(raw_error));
        assert!(!strings.check_failed_tip.trim().is_empty());

        let install_failed = deviceout_update::State {
            last_install_error: Some(raw_error.into()),
            ..check_failed
        };
        let (status, color, tip) =
            update_status_text("1.1.0", None, &install_failed, false, strings);
        assert_eq!(status, strings.install_failed);
        assert_eq!(color, theme::RED);
        assert_eq!(tip.as_deref(), Some(strings.install_failed_tip));
        assert_ne!(tip.as_deref(), Some(raw_error));
        assert!(!strings.install_failed_tip.trim().is_empty());

        let (status, _, tip) = update_status_text("1.1.0", None, &install_failed, true, strings);
        assert_eq!(status, strings.checking_status);
        assert!(tip.is_none());

        if lang != deviceout_i18n::Lang::En {
            let english = deviceout_i18n::Lang::En.strings();
            assert_ne!(strings.check_failed_tip, english.check_failed_tip);
            assert_ne!(strings.install_failed_tip, english.install_failed_tip);
        }
    }
}

#[test]
fn feedback_diagnostics_preserve_the_original_engine_error() {
    let snapshot = failed_snapshot();
    let diagnostics = session_diag(Some(&snapshot), "USB DAC");
    assert!(diagnostics.contains("engine_error=设备 8 声道，DAW 2 声道，不一致"));
}

fn collect_text(shape: egui::Shape, text: &mut Vec<egui::epaint::TextShape>) {
    match shape {
        egui::Shape::Text(shape) => text.push(shape),
        egui::Shape::Vec(shapes) => {
            for shape in shapes {
                collect_text(shape, text);
            }
        }
        _ => {}
    }
}

#[test]
fn long_device_names_stay_within_the_card() {
    for name in [
        "USB Audio Interface with a very long manufacturer and model name (Output channels 1 and 2)",
        "長いデバイス名のオーディオインターフェース（ステレオ出力チャンネル１と２）",
        "超长设备名称的音频接口（专业录音设备的立体声输出通道一和二）",
    ] {
        let mut harness = DeviceCardHarness::new(name);
        for width in [240.0, 440.0, 640.0] {
            let (card, text) = harness.frame(width, Vec::new());
            assert!(card.right() <= width - 18.0 + 1.0, "{name}: {card:?}");
            let selected = text.iter().find(|text| text.galley.job.text == name).unwrap();
            assert_eq!(selected.galley.rows.len(), 1);
            let text_right = selected.pos.x + selected.galley.rect.right();
            let refresh_left = card.right() - 14.0 - 30.0;
            assert!(text_right < refresh_left, "{name}: {text_right}");
            if width == 240.0 {
                assert!(selected.galley.elided);
            }
        }
    }
}

#[test]
fn short_device_names_are_not_elided() {
    let name = "USB DAC";
    let mut harness = DeviceCardHarness::new(name);
    let (_, text) = harness.frame(440.0, Vec::new());
    let selected = text
        .iter()
        .find(|text| text.galley.job.text == name)
        .unwrap();
    assert!(!selected.galley.elided);
}

#[test]
fn hovering_a_truncated_device_name_shows_the_full_name() {
    let name = "USB Audio Interface with a very long manufacturer and model name (Output channels 1 and 2)";
    let mut harness = DeviceCardHarness::new(name);
    let (_, text) = harness.frame(440.0, Vec::new());
    let selected = text
        .iter()
        .find(|text| text.galley.job.text == name)
        .unwrap();
    assert!(selected.galley.elided);
    let pointer = selected.pos + egui::vec2(5.0, 5.0);
    harness.frame(440.0, vec![egui::Event::PointerMoved(pointer)]);
    harness.time += 1.0;
    harness.frame(440.0, Vec::new());
    let (_, text) = harness.frame(440.0, Vec::new());
    let tooltip = text
        .iter()
        .find(|text| text.galley.job.text == name && !text.galley.elided)
        .expect("悬浮提示应显示完整设备名称");
    assert!(tooltip.pos.x >= 0.0);
    assert!(tooltip.pos.x + tooltip.galley.rect.right() <= 440.0);
}

#[test]
fn device_menu_wraps_long_names_without_widening_the_window() {
    let name = "USB Audio Interface with a very long manufacturer and model name (Output channels 1 and 2)";
    let mut harness = DeviceCardHarness::new(name);
    let width = 440.0;
    let (_, text) = harness.frame(width, Vec::new());
    let selected = text
        .iter()
        .find(|text| text.galley.job.text == name)
        .unwrap();
    let pointer = selected.pos + egui::vec2(5.0, 5.0);
    harness.frame(
        width,
        vec![
            egui::Event::PointerMoved(pointer),
            egui::Event::PointerButton {
                pos: pointer,
                button: egui::PointerButton::Primary,
                pressed: true,
                modifiers: egui::Modifiers::NONE,
            },
        ],
    );
    harness.frame(
        width,
        vec![egui::Event::PointerButton {
            pos: pointer,
            button: egui::PointerButton::Primary,
            pressed: false,
            modifiers: egui::Modifiers::NONE,
        }],
    );
    let (_, text) = harness.frame(width, Vec::new());
    let option = text
        .iter()
        .find(|text| text.galley.job.text == name && !text.galley.elided)
        .expect("展开的菜单应显示完整设备名称");
    assert!(option.galley.rows.len() > 1);
    assert!(
        option.pos.x + option.galley.rect.right() <= width,
        "菜单文字位置 {:?}，边界 {:?}",
        option.pos,
        option.galley.rect
    );
}
