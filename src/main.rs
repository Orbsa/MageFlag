use std::sync::{Arc, Mutex};
use std::thread;
use std::time::Duration;

use arboard::Clipboard;
use chrono::{DateTime, Local};
use eframe::{App, CreationContext, egui};
use image::{DynamicImage, GenericImageView, Pixel, RgbaImage, imageops::FilterType};
use palette::{FromColor, Lab, Srgb};
use winreg::enums::{HKEY_CURRENT_USER, RegType};
use winreg::{RegKey, RegValue};

// === CONFIG ===
const IMAGE_WIDTH: u32 = 100;
const IMAGE_HEIGHT: u32 = 66;
const PALETTE_COLS: u32 = 7;
const PALETTE_ROWS: u32 = 6;

const REGISTRY_PATH: &str = "Software\\jrsjams\\MageArena";
const REGISTRY_VALUE_NAME: &str = "flagGrid_h3042110417";

const EMBEDDED_PALETTE: &[u8] = include_bytes!("palette.png");

// === UI STATE ===
#[derive(Default)]
struct AppState {
    last_update: Option<String>,
    quit_requested: bool,
}

struct MageFlagApp {
    state: Arc<Mutex<AppState>>,
}

impl App for MageFlagApp {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        let mut state = self.state.lock().unwrap();

        egui::CentralPanel::default().show(ctx, |ui| {
            ui.heading("📋 Clipboard Watcher");
            ui.label(
                "This tool watches your clipboard for images and writes them to the registry.",
            );
            if let Some(ref status) = state.last_update {
                ui.label(format!("✅ Last update: {status}"));
            } else {
                ui.label("No clipboard image captured yet.");
            }

            ui.add_space(10.0);
            if ui.button("Quit").clicked() {
                state.quit_requested = true;
            }
        });

        if state.quit_requested {
            std::process::exit(0);
        }

        ctx.request_repaint_after(Duration::from_millis(250));
    }
}

// === MAIN ENTRYPOINT ===
fn main() -> eframe::Result<()> {
    let state = Arc::new(Mutex::new(AppState::default()));
    let ui_state = Arc::clone(&state);
    let palette_image =
        image::load_from_memory(EMBEDDED_PALETTE).expect("Invalid embedded palette");
    let palette = sample_palette(&palette_image);

    // Spawn clipboard watcher thread
    thread::spawn(move || {
        let hkcu = RegKey::predef(HKEY_CURRENT_USER);
        let (key, _) = hkcu
            .create_subkey(REGISTRY_PATH)
            .expect("Failed to open registry key");

        let mut clipboard = Clipboard::new().unwrap();
        let mut last_hash: u64 = 0;

        loop {
            if let Ok(image) = clipboard.get_image() {
                let current_hash = calculate_image_hash(&image.bytes);
                if current_hash != last_hash {
                    last_hash = current_hash;

                    let raw = RgbaImage::from_raw(
                        image.width as u32,
                        image.height as u32,
                        image.bytes.to_vec(),
                    )
                    .expect("Invalid clipboard image");

                    let resized: DynamicImage = DynamicImage::ImageRgba8(raw).resize_exact(
                        IMAGE_WIDTH,
                        IMAGE_HEIGHT,
                        FilterType::Nearest,
                    );

                    let csv = encode_uv_csv(&resized, &palette);
                    let reg_value = RegValue {
                        vtype: RegType::REG_BINARY,
                        bytes: csv.into_bytes(),
                    };

                    key.set_raw_value(REGISTRY_VALUE_NAME, &reg_value)
                        .expect("Failed to write to registry");

                    let mut state = state.lock().unwrap();

                    let now = std::time::SystemTime::now();
                    let now_local: DateTime<Local> = now.into();
                    state.last_update = Some(now_local.format("%Y-%m-%d %H:%M:%S").to_string());
                }
            }

            {
                let state = state.lock().unwrap();
                if state.quit_requested {
                    break;
                }
            }

            thread::sleep(Duration::from_secs(1));
        }
    });

    let native_options = eframe::NativeOptions {
        viewport: egui::viewport::ViewportBuilder::default()
            .with_inner_size([400.0, 160.0])
            .with_title("MageFlag Clipboard Watcher"),
        ..Default::default()
    };

    eframe::run_native(
        "MageFlag Clipboard Watcher",
        native_options,
        Box::new(|_cc: &CreationContext| Box::new(MageFlagApp { state: ui_state })),
    )
}

// === SUPPORT ===

fn calculate_image_hash(data: &[u8]) -> u64 {
    use std::collections::hash_map::DefaultHasher;
    use std::hash::{Hash, Hasher};

    let mut hasher = DefaultHasher::new();
    data.hash(&mut hasher);
    hasher.finish()
}

fn sample_palette(img: &DynamicImage) -> Vec<[u8; 3]> {
    let (w, h) = img.dimensions();
    let cell_w = w as f32 / PALETTE_COLS as f32;
    let cell_h = h as f32 / PALETTE_ROWS as f32;

    let mut colors = Vec::with_capacity((PALETTE_COLS * PALETTE_ROWS) as usize);

    for row in 0..PALETTE_ROWS {
        for col in 0..PALETTE_COLS {
            let cx = ((col as f32 + 0.5) * cell_w).round() as u32;
            let cy = ((row as f32 + 0.5) * cell_h).round() as u32;
            let pixel = average_patch(img, cx.min(w - 1), cy.min(h - 1));
            colors.push(pixel);
        }
    }

    colors
}

fn average_patch(img: &DynamicImage, cx: u32, cy: u32) -> [u8; 3] {
    let mut r = 0u32;
    let mut g = 0u32;
    let mut b = 0u32;
    let mut count = 0u32;

    for dx in -1..=1 {
        for dy in -1..=1 {
            let x = (cx as i32 + dx).clamp(0, img.width() as i32 - 1) as u32;
            let y = (cy as i32 + dy).clamp(0, img.height() as i32 - 1) as u32;
            let pixel = img.get_pixel(x, y).to_rgb();
            r += pixel[0] as u32;
            g += pixel[1] as u32;
            b += pixel[2] as u32;
            count += 1;
        }
    }

    [(r / count) as u8, (g / count) as u8, (b / count) as u8]
}

fn encode_uv_csv(img: &DynamicImage, palette: &[[u8; 3]]) -> String {
    let mut result = Vec::with_capacity((IMAGE_WIDTH * IMAGE_HEIGHT) as usize);

    for x in 0..IMAGE_WIDTH {
        for y in (0..IMAGE_HEIGHT).rev() {
            let pixel = img.get_pixel(x, y);
            let rgb = [pixel[0], pixel[1], pixel[2]];

            let (idx, _) = palette
                .iter()
                .enumerate()
                .map(|(i, color)| (i, lab_distance(rgb, *color)))
                .min_by(|a, b| a.1.partial_cmp(&b.1).unwrap())
                .unwrap();

            let raw_row = idx as u32 / PALETTE_COLS;
            let row = PALETTE_ROWS - 1 - raw_row;
            let col = idx as u32 % PALETTE_COLS;

            let u = (col as f32 + 0.5) / PALETTE_COLS as f32;
            let v = (row as f32 + 0.5) / PALETTE_ROWS as f32;

            result.push(format!("{u:.2}:{v:.2}"));
        }
    }

    result.join(",")
}

fn lab_distance(a: [u8; 3], b: [u8; 3]) -> f32 {
    let lab_a: Lab = Lab::from_color(Srgb::new(a[0], a[1], a[2]).into_format());
    let lab_b: Lab = Lab::from_color(Srgb::new(b[0], b[1], b[2]).into_format());

    let dl = lab_a.l - lab_b.l;
    let da = lab_a.a - lab_b.a;
    let db = lab_a.b - lab_b.b;

    (dl * dl + da * da + db * db).sqrt()
}
