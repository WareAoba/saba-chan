fn main() {
    if cfg!(target_os = "windows") {
        let mut res = winres::WindowsResource::new();
        res.set_icon("icon.ico");
        res.set("ProductName", "Saba-chan CLI");
        res.set("FileDescription", "Saba-chan Game Server Manager CLI");
        res.compile().expect("Failed to compile Windows resources");
    }
}
