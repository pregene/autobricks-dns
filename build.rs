fn main() {
    println!("cargo:rerun-if-changed=VERSION");
    let version = include_str!("VERSION").trim();
    if !valid_version(version) {
        eprintln!("invalid product version; expected M.m.build: {version}");
        std::process::exit(1);
    }
    if version != env!("CARGO_PKG_VERSION") {
        eprintln!("VERSION and Cargo.toml package version must match");
        std::process::exit(1);
    }
    println!("cargo:rustc-env=AUTOBRICKS_PRODUCT_VERSION={version}");
}

fn valid_version(version: &str) -> bool {
    let fields: Vec<&str> = version.split('.').collect();
    fields.len() == 3
        && fields
            .iter()
            .all(|field| !field.is_empty() && field.chars().all(|value| value.is_ascii_digit()))
}
