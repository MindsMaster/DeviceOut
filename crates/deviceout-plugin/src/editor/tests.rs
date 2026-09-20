use super::*;

struct DeviceCardHarness {
    ctx: egui::Context,
    wiring: Wiring,
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
                    device_card(ui, &self.wiring, None);
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
