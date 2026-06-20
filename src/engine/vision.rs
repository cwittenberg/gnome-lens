use std::path::Path;
use std::process::Command;
use image::DynamicImage;
use super::smart_extract::SmartExtractor;

pub struct VisionEngine {
    smart_extractor: SmartExtractor,
}

impl VisionEngine {
    pub fn new() -> Self {
        Self {
            smart_extractor: SmartExtractor::new(),
        }
    }

    /// Primary entry point for analyzing images and extracting embedded text or data natively.
    pub fn process_image(&self, path: &str) -> serde_json::Value {
        if !Path::new(path).exists() {
            return serde_json::json!({
                "type": "error",
                "text": "",
                "confidence": 0.0,
                "entities": []
            });
        }

        // 1. Native QR Code Scanning pass (100% Pure Rust via rqrr)
        let qr_data = self.extract_qr(path);
        if !qr_data.is_empty() {
            return serde_json::json!({
                "type": "qr",
                "text": qr_data,
                "confidence": 1.0,
                "entities": self.smart_extractor.extract_entities(&qr_data)
            });
        }

        // 2. Load and analyze the image matrix for visual characteristics
        let img = match image::open(path) {
            Ok(opened) => opened,
            Err(_) => {
                return serde_json::json!({
                    "type": "error",
                    "text": "Failed to open or parse image matrix",
                    "confidence": 0.0,
                    "entities": []
                });
            }
        };

        let brightness = self.calculate_mean_brightness(&img);
        
        // 3. Process OCR via Daemon Process (EGO Compliant)
        // Bypasses Gio.Subprocess restrictions since this executes outside the GNOME shell thread.
        let mut processed_text = self.extract_text_via_tesseract(path);
        let mut confidence = self.score_text_quality(&processed_text);

        // Multi-pass validation: If low text quality and image is dark, invert natively using Rust and scan again
        if confidence < 0.4 && brightness < 0.45 {
            let mut inverted_img = img.to_luma8();
            image::imageops::invert(&mut inverted_img);
            
            let temp_path = "/tmp/gnome_lens_inverted_tmp.png";
            if inverted_img.save(temp_path).is_ok() {
                let second_pass_text = self.extract_text_via_tesseract(temp_path);
                let second_pass_confidence = self.score_text_quality(&second_pass_text);
                
                if second_pass_confidence > confidence {
                    processed_text = second_pass_text;
                    confidence = second_pass_confidence;
                }
                let _ = std::fs::remove_file(temp_path);
            }
        }

        let entities = self.smart_extractor.extract_entities(processed_text.trim());

        serde_json::json!({
            "type": "ocr",
            "text": processed_text.trim(),
            "confidence": confidence,
            "entities": entities
        })
    }

    /// Scans the image for valid QR matrix specifications using pure Rust decoding
    fn extract_qr(&self, path: &str) -> String {
        if let Ok(img) = image::open(path) {
            let gray_img = img.to_luma8();
            let mut prepared = rqrr::PreparedImage::prepare(gray_img);
            let grids = prepared.detect_grids();
            for grid in grids {
                if let Ok((_meta, decoded_content)) = grid.decode() {
                    return decoded_content;
                }
            }
        }
        String::new()
    }

    /// Evaluates the mean light level of the pixel buffer to detect dark themes or screenshots
    fn calculate_mean_brightness(&self, img: &DynamicImage) -> f64 {
        let luma = img.to_luma8();
        let pixels = luma.as_raw();
        if pixels.is_empty() {
            return 0.0;
        }
        let total: u64 = pixels.iter().map(|&p| p as u64).sum();
        (total as f64) / (pixels.len() as f64) / 255.0
    }

    /// Executes tesseract natively from the Rust Daemon using std::process
    fn extract_text_via_tesseract(&self, path: &str) -> String {
        if let Ok(output) = Command::new("tesseract")
            .arg(path)
            .arg("stdout")
            .arg("-l")
            .arg("eng")
            .output() 
        {
            if output.status.success() {
                let mut text = String::from_utf8_lossy(&output.stdout).into_owned();
                // Strip excessive hallucinated newlines from empty areas
                while text.contains("\n\n\n") {
                    text = text.replace("\n\n\n", "\n\n");
                }
                return text;
            }
        }
        String::new()
    }

    /// Replicates the garbage/density validation ratio scoring system
    fn score_text_quality(&self, text: &str) -> f64 {
        if text.is_empty() {
            return 0.0;
        }
        let total_chars = text.len();
        let alphanumeric_chars = text.chars().filter(|c| c.is_alphanumeric() || c.is_whitespace()).count();
        (alphanumeric_chars as f64) / (total_chars as f64)
    }
}