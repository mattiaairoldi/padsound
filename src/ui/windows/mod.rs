mod live;
mod setup;

use std::path::PathBuf;
use std::time::Duration;

use anyhow::{Result, anyhow};
use eframe::egui;

use crate::runtime::PadsoundRuntime;

const REPAINT_INTERVAL: Duration = Duration::from_millis(50);

pub fn run(
    initial_config: PathBuf,
    preferred_audio: Option<String>,
    preferred_midi: Option<String>,
    disable_midi: bool,
) -> Result<()> {
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_inner_size([1180.0, 760.0])
            .with_min_inner_size([760.0, 520.0]),
        renderer: eframe::Renderer::Glow,
        centered: true,
        ..Default::default()
    };

    eframe::run_native(
        "Padsound",
        options,
        Box::new(move |_creation_context| {
            Ok(Box::new(PadsoundGui::new(
                initial_config,
                preferred_audio,
                preferred_midi,
                disable_midi,
            )))
        }),
    )
    .map_err(|error| anyhow!("failed to start the Windows GUI: {error}"))
}

pub fn show_error(message: &str) {
    use windows_sys::Win32::UI::WindowsAndMessaging::{MB_ICONERROR, MB_OK, MessageBoxW};

    let text = wide(message);
    let caption = wide("Padsound error");
    unsafe {
        MessageBoxW(
            std::ptr::null_mut(),
            text.as_ptr(),
            caption.as_ptr(),
            MB_OK | MB_ICONERROR,
        );
    }
}

struct PadsoundGui {
    config_path: String,
    audio_devices: Vec<String>,
    midi_devices: Vec<String>,
    selected_audio: Option<String>,
    selected_midi: Option<String>,
    disable_midi: bool,
    runtime: Option<PadsoundRuntime>,
    error: Option<String>,
}

impl PadsoundGui {
    fn new(
        initial_config: PathBuf,
        preferred_audio: Option<String>,
        preferred_midi: Option<String>,
        disable_midi: bool,
    ) -> Self {
        let mut gui = Self {
            config_path: initial_config.to_string_lossy().into_owned(),
            audio_devices: Vec::new(),
            midi_devices: Vec::new(),
            selected_audio: preferred_audio,
            selected_midi: preferred_midi,
            disable_midi,
            runtime: None,
            error: None,
        };
        gui.refresh_devices();
        gui
    }
}

impl eframe::App for PadsoundGui {
    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        ui.ctx().request_repaint_after(REPAINT_INTERVAL);
        if self.runtime.is_some() {
            self.show_running(ui);
        } else {
            self.show_setup(ui);
        }
    }
}

fn wide(text: &str) -> Vec<u16> {
    text.encode_utf16().chain(std::iter::once(0)).collect()
}
