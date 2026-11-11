//! `praxis-app`: the Praxis Sound desktop app. On Windows release builds there is no console
//! window.

#![forbid(unsafe_code)]
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

fn main() {
    praxis_app::run();
}
