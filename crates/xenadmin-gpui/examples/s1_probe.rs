//! Spike S1 probe.
//!
//! Binds a `wl_keyboard` on `WAYLAND_DISPLAY` when that variable is set.
//! That is an own connection, not the foreign display route A needs, and it
//! does not prove a guest received the keys. With no display, the process
//! exits 2.

fn main() {
    let display = std::env::var("WAYLAND_DISPLAY").unwrap_or_default();
    println!("wayland_display={display}");
    let mut keyboard = match xenadmin_gpui::ForeignKeyboard::connect_own() {
        Ok(keyboard) => keyboard,
        Err(error) => {
            println!("own_connection=error");
            println!("{error}");
            std::process::exit(2);
        }
    };
    if let Err(error) = keyboard.roundtrip_own() {
        println!("roundtrip=error");
        println!("{error}");
        std::process::exit(2);
    }
    println!("keyboard_bound={}", keyboard.keyboard_bound());
    match keyboard.dispatch_pending() {
        Ok(events) => println!("pending_key_events={}", events.len()),
        Err(error) => {
            println!("dispatch=error");
            println!("{error}");
            std::process::exit(2);
        }
    }
}
