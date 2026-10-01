fn main() {
    println!("cargo:rerun-if-changed=TreeMapicon.ico");

    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() != Ok("windows") {
        return;
    }

    let mut resources = winres::WindowsResource::new();
    resources
        .set_icon("TreeMapicon.ico")
        .set("ProductName", "TreeMap")
        .set("FileDescription", "TreeMap disk space explorer")
        .set("OriginalFilename", "TreeMap.exe");
    resources
        .compile()
        .expect("failed to embed TreeMap Windows icon and version information");
}
