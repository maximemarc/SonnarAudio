//! MixFlow — point d'entrée multi-plateforme.
//!
//! - **Windows** : moteur audio maison (cpal/WASAPI + VB-Cable), voir
//!   `windows_app.rs`, `audio/` et `winapps.rs`.
//! - **Linux** : PipeWire fait le mixage ; MixFlow pilote une chaîne de
//!   filtres (volume + EQ paramétrique) par application. Voir `linux/`.

#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

#[cfg(windows)]
mod audio;
#[cfg(windows)]
mod winapps;
#[cfg(windows)]
mod windows_app;

#[cfg(target_os = "linux")]
mod linux;

fn main() {
    #[cfg(windows)]
    windows_app::run();
    #[cfg(target_os = "linux")]
    linux::run();
}
