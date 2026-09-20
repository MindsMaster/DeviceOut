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

    fn click(&mut self, width: f32, pos: egui::Pos2) {
        self.frame(
            width,
            vec![
                egui::Event::PointerMoved(pos),
                egui::Event::PointerButton {
                    pos,
                    button: egui::PointerButton::Primary,
                    pressed: true,
                    modifiers: egui::Modifiers::NONE,
                },
            ],
        );
        self.frame(
            width,
            vec![egui::Event::PointerButton {
                pos,
                button: egui::PointerButton::Primary,
                pressed: false,
                modifiers: egui::Modifiers::NONE,
            }],
        );
    }

    fn open_menu(&mut self, width: f32, name: &str) -> Vec<egui::epaint::TextShape> {
        let (_, text) = self.frame(width, Vec::new());
        let selected = text
            .iter()
            .find(|text| text.galley.job.text == name)
            .unwrap();
        self.click(width, selected.pos + egui::vec2(5.0, 5.0));
        self.frame(width, Vec::new()).1
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
            let refresh_left = card.right() - 14.0 - 34.0;
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
fn hovering_a_short_device_name_does_not_show_a_tooltip() {
    let name = "USB DAC";
    let mut harness = DeviceCardHarness::new(name);
    let (_, text) = harness.frame(440.0, Vec::new());
    let selected = text
        .iter()
        .find(|text| text.galley.job.text == name)
        .unwrap();
    let pointer = selected.pos + egui::vec2(5.0, 5.0);
    harness.frame(440.0, vec![egui::Event::PointerMoved(pointer)]);
    harness.time += 1.0;
    harness.frame(440.0, Vec::new());
    let (_, text) = harness.frame(440.0, Vec::new());
    assert_eq!(
        text.iter()
            .filter(|text| text.galley.job.text == name)
            .count(),
        1
    );
}

#[test]
fn an_open_device_menu_never_shows_duplicate_name_tooltips() {
    let name = "USB Audio Interface with a very long manufacturer and model name (Output channels 1 and 2)";
    let mut harness = DeviceCardHarness::new(name);
    let text = harness.open_menu(440.0, name);
    let option = text
        .iter()
        .find(|text| text.galley.job.text == name && !text.galley.elided)
        .unwrap();
    let option_pointer = option.pos + egui::vec2(5.0, 5.0);
    let selected = text
        .iter()
        .find(|text| text.galley.job.text == name && text.galley.elided)
        .unwrap();
    let selected_pointer = selected.pos + egui::vec2(5.0, 5.0);

    for pointer in [option_pointer, selected_pointer] {
        harness.frame(440.0, vec![egui::Event::PointerMoved(pointer)]);
        harness.time += 1.0;
        harness.frame(440.0, Vec::new());
        let (_, text) = harness.frame(440.0, Vec::new());
        assert_eq!(
            text.iter()
                .filter(|text| text.galley.job.text == name)
                .count(),
            2
        );
    }

    harness.click(440.0, option_pointer);
    let (_, text) = harness.frame(440.0, Vec::new());
    assert_eq!(
        text.iter()
            .filter(|text| text.galley.job.text == name)
            .count(),
        1
    );
    assert_eq!(*harness.wiring.params.device_id.read(), "test-device");
}

#[test]
fn device_options_align_labels_and_distinguish_selection_from_hover() {
    let ctx = egui::Context::default();
    theme::install(&ctx);
    let names = [
        "USB DAC",
        "USB Audio Interface with a very long manufacturer and model name (Output channels 1 and 2)",
    ];
    let mut rows = Vec::new();
    let mut draw = |events| {
        rows.clear();
        let output = ctx.run(
            egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(
                    egui::Pos2::ZERO,
                    egui::vec2(300.0, 560.0),
                )),
                events,
                ..Default::default()
            },
            |ctx| {
                egui::CentralPanel::default().show(ctx, |ui| {
                    ui.spacing_mut().item_spacing.y = 2.0;
                    rows.push(widgets::device_option(ui, names[0], true));
                    rows.push(widgets::device_option(ui, names[1], false));
                });
            },
        );
        (output, rows.clone())
    };
    let (_, rows) = draw(Vec::new());
    let (output, rows) = draw(vec![egui::Event::PointerMoved(rows[1].rect.center())]);
    assert!(!rows[0].hovered());
    assert!(rows[1].hovered());
    assert_eq!(rows[0].rect.width(), rows[1].rect.width());
    assert!(rows[1].rect.height() > rows[0].rect.height());
    assert!((rows[1].rect.top() - rows[0].rect.bottom() - 2.0).abs() < 0.1);

    let shapes: Vec<_> = output.shapes.iter().map(|clipped| &clipped.shape).collect();
    let checkmarks = shapes.iter().filter(|shape| {
        matches!(shape, egui::Shape::Path(path) if path.points.len() == 3 && path.stroke.width == 1.6)
    }).count();
    assert_eq!(checkmarks, 1);
    for (row, fill) in rows.iter().zip([theme::MENU_SELECTED, theme::MENU_HOVER]) {
        assert!(shapes.iter().any(|shape| {
            matches!(shape, egui::Shape::Rect(rect)
                if rect.rect == row.rect && rect.fill == fill && rect.corner_radius == theme::CORNER_SMALL)
        }));
    }
    assert!(!shapes
        .iter()
        .any(|shape| { matches!(shape, egui::Shape::LineSegment { .. }) }));
    assert!(shapes.iter().any(|shape| {
        matches!(shape, egui::Shape::Path(path)
            if path.stroke.width == 1.6 && path.points.iter().all(|point| point.x > rows[0].rect.right() - 20.0))
    }));
    let mut text = Vec::new();
    for clipped in output.shapes {
        collect_text(clipped.shape, &mut text);
    }
    assert_eq!(text[0].pos.x, text[1].pos.x);
    assert!(!text[1].galley.elided);
    assert!(text[1].galley.rows.len() > 1);
    assert!(text[1].pos.x + text[1].galley.rect.right() <= rows[1].rect.right() - 11.0);

    ctx.memory_mut(|memory| memory.request_focus(rows[1].id));
    let (output, rows) = draw(Vec::new());
    assert!(rows[1].has_focus());
    assert!(output.shapes.iter().any(|clipped| {
        matches!(&clipped.shape, egui::Shape::Rect(rect)
            if rect.rect == rows[1].rect.shrink(1.0) && rect.stroke.color == theme::TEXT_DIM)
    }));
    let (_, rows) = draw(vec![egui::Event::Key {
        key: egui::Key::Enter,
        physical_key: None,
        pressed: true,
        repeat: false,
        modifiers: egui::Modifiers::NONE,
    }]);
    assert!(rows[1].clicked());
}

#[test]
fn device_picker_popup_matches_trigger_width_and_stays_inside_the_window() {
    fn popup_rect(shape: &egui::Shape) -> Option<egui::Rect> {
        match shape {
            egui::Shape::Rect(rect) if rect.fill == theme::MENU => Some(rect.rect),
            egui::Shape::Vec(shapes) => shapes.iter().find_map(popup_rect),
            _ => None,
        }
    }

    for (width, height, top) in [
        (240.0, 560.0, 0.0),
        (440.0, 560.0, 0.0),
        (440.0, 360.0, 260.0),
    ] {
        let ctx = egui::Context::default();
        theme::install(&ctx);
        let mut time = 0.0;
        let mut draw = |events| {
            time += 0.1;
            let mut trigger = egui::Rect::NOTHING;
            let output = ctx.run(
                egui::RawInput {
                    screen_rect: Some(egui::Rect::from_min_size(
                        egui::Pos2::ZERO,
                        egui::vec2(width, height),
                    )),
                    time: Some(time),
                    events,
                    ..Default::default()
                },
                |ctx| {
                    egui::CentralPanel::default().show(ctx, |ui| {
                        ui.add_space(top);
                        trigger = widgets::device_picker(ui, "USB DAC", |ui| {
                            for index in 0..20 {
                                widgets::device_option(
                                    ui,
                                    &format!("Audio output {index}"),
                                    index == 0,
                                );
                            }
                        })
                        .rect;
                    });
                },
            );
            (trigger, output)
        };
        let (trigger, _) = draw(Vec::new());
        let pointer = trigger.center();
        draw(vec![
            egui::Event::PointerMoved(pointer),
            egui::Event::PointerButton {
                pos: pointer,
                button: egui::PointerButton::Primary,
                pressed: true,
                modifiers: egui::Modifiers::NONE,
            },
        ]);
        draw(vec![egui::Event::PointerButton {
            pos: pointer,
            button: egui::PointerButton::Primary,
            pressed: false,
            modifiers: egui::Modifiers::NONE,
        }]);
        let (_, output) = draw(Vec::new());
        let popup = output
            .shapes
            .iter()
            .find_map(|clipped| popup_rect(&clipped.shape))
            .expect("设备浮层应可见");
        assert!(
            (popup.width() - trigger.width()).abs() < 1.0,
            "{popup:?} {trigger:?}"
        );
        assert!((popup.left() - trigger.left()).abs() < 1.0);
        assert!(popup.top() >= 0.0 && popup.bottom() <= height);
        if top == 0.0 {
            assert!((popup.top() - trigger.bottom() - 5.0).abs() < 1.0);
        } else {
            assert!((trigger.top() - popup.bottom() - 5.0).abs() < 1.0);
        }
        draw(vec![egui::Event::Key {
            key: egui::Key::Escape,
            physical_key: None,
            pressed: true,
            repeat: false,
            modifiers: egui::Modifiers::NONE,
        }]);
        let (_, output) = draw(Vec::new());
        assert!(output
            .shapes
            .iter()
            .all(|clipped| popup_rect(&clipped.shape).is_none()));
    }
}

#[test]
fn device_picker_supports_keyboard_open_and_selection() {
    let ctx = egui::Context::default();
    theme::install(&ctx);
    let mut chosen = 0;
    let mut time = 0.0;
    let mut draw = |events| {
        time += 0.1;
        let mut trigger = None;
        let mut options = Vec::new();
        let _ = ctx.run(
            egui::RawInput {
                time: Some(time),
                events,
                ..Default::default()
            },
            |ctx| {
                egui::CentralPanel::default().show(ctx, |ui| {
                    trigger = Some(widgets::device_picker(ui, "USB DAC", |ui| {
                        for (index, label) in ["USB DAC", "Headphones"].iter().enumerate() {
                            let response = widgets::device_option(ui, label, chosen == index);
                            if response.clicked() {
                                chosen = index;
                                ui.memory_mut(|memory| memory.close_popup());
                            }
                            options.push(response);
                        }
                    }));
                });
            },
        );
        (trigger.unwrap(), options, chosen)
    };
    let enter = egui::Event::Key {
        key: egui::Key::Enter,
        physical_key: None,
        pressed: true,
        repeat: false,
        modifiers: egui::Modifiers::NONE,
    };
    let (trigger, _, _) = draw(Vec::new());
    ctx.memory_mut(|memory| memory.request_focus(trigger.id));
    draw(vec![enter.clone()]);
    let (_, options, _) = draw(Vec::new());
    assert_eq!(options.len(), 2);
    ctx.memory_mut(|memory| memory.request_focus(options[1].id));
    let (_, _, chosen) = draw(vec![enter]);
    assert_eq!(chosen, 1);
    let (_, options, _) = draw(Vec::new());
    assert!(options.is_empty());
}

#[test]
fn clicking_outside_the_device_menu_closes_it() {
    let name = "USB DAC";
    let mut harness = DeviceCardHarness::new(name);
    harness.open_menu(440.0, name);
    harness.click(440.0, egui::pos2(420.0, 540.0));
    let (_, text) = harness.frame(440.0, Vec::new());
    assert_eq!(
        text.iter()
            .filter(|text| text.galley.job.text == name)
            .count(),
        1
    );
}

#[test]
fn device_menu_wraps_long_names_without_widening_the_window() {
    let name = "USB Audio Interface with a very long manufacturer and model name (Output channels 1 and 2)";
    let mut harness = DeviceCardHarness::new(name);
    let width = 440.0;
    let text = harness.open_menu(width, name);
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
