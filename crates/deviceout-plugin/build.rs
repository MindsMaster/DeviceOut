fn main() {
    let version = env!("CARGO_PKG_VERSION");
    let mut res = winres::WindowsResource::new();
    res.set("FileVersion", version);
    res.set("ProductVersion", version);
    res.set("ProductName", "DeviceOut");
    res.set("FileDescription", "DeviceOut");
    res.set("LegalCopyright", "DeviceOut");
    res.set("OriginalFilename", "DeviceOut.vst3");
    if let Err(e) = res.compile() {
        println!("cargo:warning=winres: {e}");
    }
}
