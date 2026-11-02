//! The details window, run as a second process of the same executable
//! (`kennel-spike window`), with `eframe` on glow, or on wgpu when
//! `SPIKE_RENDERER=wgpu`, to compare their memory. It redraws about once a
//! second and exits when closed.

use std::time::Duration;

use crate::log;

pub fn run() {
    let renderer = match std::env::var("SPIKE_RENDERER").as_deref() {
        Ok("wgpu") => eframe::Renderer::Wgpu,
        _ => eframe::Renderer::Glow,
    };
    log(format!(
        "window process, pid {}, renderer {renderer:?}",
        std::process::id()
    ));
    let options = eframe::NativeOptions {
        renderer,
        viewport: eframe::egui::ViewportBuilder::default()
            .with_inner_size([520.0, 320.0])
            .with_title("Kennel spike: details"),
        ..Default::default()
    };
    let mut redraws: u64 = 0;
    let result = eframe::run_ui_native("Kennel spike", options, move |ui, _frame| {
        redraws += 1;
        ui.heading("Kennel spike: details window");
        ui.label(format!("A second process, pid {}.", std::process::id()));
        ui.label(format!("Redraws so far: {redraws}, about one a second."));
        ui.label("Close this window: the tray logs that the process exited.");
        ui.ctx().request_repaint_after(Duration::from_secs(1));
    });
    if let Err(error) = result {
        log(format!("the window failed: {error}"));
    }
    log("window process exits");
}
