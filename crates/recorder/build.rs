fn main() {
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() != Ok("windows") {
        return;
    }
    println!("cargo:rerun-if-changed=../../assets/tgow-recorder.ico");
    let mut res = winresource::WindowsResource::new();
    res.set_icon("../../assets/tgow-recorder.ico")
        .set("ProductName", "TGOW Recorder")
        .set("FileDescription", "TGOW Recorder capture engine")
        .set("CompanyName", "T-GOW")
        .set("LegalCopyright", "Copyright 2026 T-GOW");
    if let Err(err) = res.compile() {
        println!("cargo:warning=could not embed the Windows resources: {err}");
    }
}
