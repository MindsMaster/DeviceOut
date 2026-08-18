use deviceout_sink::wasapi::{list_output_devices, ComGuard};

fn main() {
    let _com = ComGuard::new().expect("COM 初始化失败");
    let devices = list_output_devices().expect("枚举设备失败");

    println!("共 {} 个活动输出设备\n", devices.len());
    for d in &devices {
        println!("{d}");
        println!("   id = {}\n", d.id);
    }
}
