use deviceout_core::{DriftResampler, DriftTuning};

const CHANNELS: usize = 2;
const OUTPUT_FRAMES: usize = 480;
const PERIODS: usize = 12;

fn main() {
    let tuning = DriftTuning::default();
    let mut r = DriftResampler::new(CHANNELS, OUTPUT_FRAMES, 1.0, &tuning).unwrap();

    println!("输出定长        {} 帧", r.output_frames());
    println!("输入上界        {} 帧", r.input_frames_max());
    println!(
        "滤波器输出延迟  {} 帧（output_delay，相位延迟）",
        r.output_delay()
    );
    println!();

    let input = vec![0.0f32; r.input_frames_max() * CHANNELS];
    let mut output = vec![0.0f32; r.output_frames() * CHANNELS];

    let mut consumed_total = 0usize;
    let mut produced_total = 0usize;

    println!(
        "{:>4}  {:>10}  {:>10}  {:>12}",
        "周期", "所需输入", "累计输入", "输入-输出"
    );
    for period in 1..=PERIODS {
        let need = r.input_frames_next();
        let consumed = r
            .process(&input[..need * CHANNELS], &mut output)
            .expect("重采样失败");
        consumed_total += consumed;
        produced_total += r.output_frames();

        println!(
            "{:>4}  {:>10}  {:>10}  {:>12}",
            period,
            need,
            consumed_total,
            consumed_total as i64 - produced_total as i64
        );
    }

    let delta = consumed_total as i64 - produced_total as i64;
    println!();
    println!("累计输入相对输出差 {delta} 帧 远小于滤波器长度");
    println!("延迟线为零填充 预填充无需为此增加余量");
}
