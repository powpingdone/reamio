fn main() {
    println!("cargo:rerun-if-changed=src/migrations");
    println!("cargo:rerun-if-changed=src/templates");
}
