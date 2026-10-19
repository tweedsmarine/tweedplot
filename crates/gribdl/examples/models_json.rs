//! Print the download dialog's model list as JSON (for UI tests).
fn main() {
    let v: Vec<_> = gribdl::Model::all().iter().map(|m| m.info()).collect();
    println!("{}", serde_json::to_string(&v).unwrap());
}
