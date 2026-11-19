// Embed the application icon and version info into autoflow.exe.
fn main() {
    println!("cargo:rerun-if-changed=../../assets/autoflowmate.ico");
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("windows") {
        let mut res = winresource::WindowsResource::new();
        res.set_icon("../../assets/autoflowmate.ico");
        res.set("ProductName", "AutoFlowMate");
        res.set("FileDescription", "AutoFlowMate command-line runner");
        res.set("InternalName", "autoflow");
        res.set("OriginalFilename", "autoflow.exe");
        res.set("Comments", "https://github.com/SRathinaGiri/AutoFlowMate");
        if let Err(e) = res.compile() {
            println!("cargo:warning=could not embed the icon: {e}");
        }
    }
}
