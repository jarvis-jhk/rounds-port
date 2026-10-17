//! Prints the objects of a `UnityFS` bundle (debugging the bundle reader).
//!
//! `cargo run --release -p unity-assets --example bundle_dump -- FILE.bundle`
fn main() {
    let path = std::env::args().nth(1).expect("bundle path");
    let data = std::fs::read(&path).expect("read");
    for e in unity_assets::bundle::read_bundle(&data).expect("bundle") {
        println!("entry {} flags {} {} bytes", e.path, e.flags, e.data.len());
        if e.flags & 4 == 0 {
            continue;
        }
        let (objs, errs) = unity_assets::bundle::read_embedded(&e.data).expect("file");
        for o in objs {
            let name = o.value.get("m_Name").str().to_owned();
            println!("  {} class {} {name}", o.path_id, o.class_id);
        }
        for e in errs {
            println!("  error {e}");
        }
    }
}
