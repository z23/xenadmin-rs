//! The client with the GPUI Kit front end. The released binary stays
//! `xenadmin-rs` (the Rune front end) until cutover; this one is built
//! beside it. Everything is in `xenadmin_gpui::app`.

fn main() -> std::process::ExitCode {
    xenadmin_gpui::app::main()
}
