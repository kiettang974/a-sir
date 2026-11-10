//! Prints what the app would see: list_memos
fn main() {
    let home = std::env::var("HOME").unwrap();
    println!(
        "{:?}",
        asr_core::voice_memos::list(&asr_core::voice_memos::folder(home.as_ref())).map(|r| r.len())
    );
}
