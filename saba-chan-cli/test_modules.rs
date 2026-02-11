use std::fs;
use std::path::Path;

fn main() {
    let modules_dir = r"C:\Git\saba-chan\modules";
    let dir = Path::new(modules_dir);
    println!("Dir exists: {}", dir.exists());
    
    for entry in fs::read_dir(dir).unwrap().flatten() {
        let toml_path = entry.path().join("module.toml");
        println!("Checking: {:?} exists={}", toml_path, toml_path.exists());
        if toml_path.exists() {
            let content = fs::read_to_string(&toml_path).unwrap();
            let table: toml::Value = content.parse().unwrap();
            let name = table.get("module").and_then(|m| m.get("name")).and_then(|v| v.as_str());
            let aliases_section = table.get("aliases");
            let protocols = table.get("protocols");
            println!("  name={:?} aliases={:?} protocols={:?}", name, aliases_section.is_some(), protocols.is_some());
        }
    }
}
