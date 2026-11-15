mod app;
mod canvas;
mod highlight;
mod layout;
mod sample;
mod tauri_bridge;
mod types;

use app::App;
use leptos::prelude::*;

fn main() {
    console_error_panic_hook::set_once();
    mount_to_body(App);
}
