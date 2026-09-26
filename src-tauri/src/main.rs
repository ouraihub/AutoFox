#![doc = "AutoFox desktop binary."]
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

fn main() {
    if let Err(error) = autofox_lib::run() {
        eprintln!("AutoFox could not start: {error}");
        std::process::exit(1);
    }
}
