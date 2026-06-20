// src/ingestion/video.rs
/*!
 * ============================================================================================
 * CTO ARCHITECTURAL ROADMAP: THE FUTURE OF VIDEO SEARCH
 * ============================================================================================
 * How far can we go with indexing movies? All the way to scene-level semantic search.
 * * CURRENT ARCHITECTURE (PHASE 1 - Implemented Below):
 * - Zero-cost extraction: We leverage `ffprobe` for dense container metadata (resolution,
 * codecs, bitrates, tags, creation times).
 * - Fast-pass Subtitles: We use `ffmpeg` to rip embedded subtitle tracks (SRT/VTT) directly 
 * into the vector database. This allows full-text RAG search on the dialogue of the film.
 * * THE MID-TERM VISION (PHASE 2 - Transcription & Chunking):
 * - Local Audio-to-Text: For movies without subtitles, we integrate a quantized `whisper.cpp` 
 * daemon. We strip the audio track via ffmpeg and transcribe it on the CPU/GPU.
 * - Temporal Chunking: We stop indexing a movie as a single document. Instead, we chunk the 
 * transcript into 3-minute overlapping windows, storing the start/end timestamps in the 
 * SQLite metadata JSON. When the user searches "show me the scene where they rob the bank", 
 * the GNOME extension can launch VLC or MPV passing `--start-time={timestamp}` to jump 
 * directly to the exact second.
 * * THE LATE-TERM VISION (PHASE 3 - Visual Semantic Analysis):
 * - Frame Sampling: We extract one frame every 5 seconds using `ffmpeg`.
 * - Multimodal Vision: We pass these frames through a local Vision-Language Model (like 
 * LLaVA or Phi-3-Vision). The model generates descriptions like "A red sports car driving 
 * in the rain at night."
 * - Dense Vector Embedding: We embed these scene descriptions into the Vector DB. 
 * Result: The user searches for "red car rain", and the RRF algorithm correlates the vector 
 * match with the frame's timestamp, instantly finding a purely visual scene with zero dialogue.
 * * THE MOONSHOT (PHASE 4 - Entity & Biometric Tagging):
 * - Local facial embeddings using dlib/OpenCV. The user tags a face once, and the engine 
 * scrapes all movies to find every timestamp that actor appears in locally.
 * ============================================================================================
 */

use std::path::Path;
use std::process::Command;
use super::FileExtractor;

pub struct VideoExtractor;

impl VideoExtractor {
    pub fn new() -> Self {
        Self
    }
}

impl FileExtractor for VideoExtractor {
    fn can_handle(&self, extension: &str) -> bool {
        matches!(extension, "mp4" | "mkv" | "avi" | "mov" | "webm" | "flv" | "wmv" | "m4v")
    }

    fn extract(&self, path: &Path) -> Result<String, String> {
        let path_str = path.to_string_lossy().to_string();
        let mut extracted_content = String::new();

        // 1. Extract Structural Metadata via FFprobe
        if let Ok(probe_output) = Command::new("ffprobe")
            .arg("-v")
            .arg("quiet")
            .arg("-print_format")
            .arg("json")
            .arg("-show_format")
            .arg("-show_streams")
            .arg(&path_str)
            .output()
        {
            if probe_output.status.success() {
                if let Ok(json) = serde_json::from_slice::<serde_json::Value>(&probe_output.stdout) {
                    extracted_content.push_str("--- VIDEO METADATA ---\n");
                    
                    if let Some(format) = json.get("format") {
                        if let Some(duration) = format.get("duration").and_then(|d| d.as_str()) {
                            extracted_content.push_str(&format!("Duration: {} seconds\n", duration));
                        }
                        if let Some(tags) = format.get("tags").and_then(|t| t.as_object()) {
                            for (k, v) in tags {
                                if let Some(val_str) = v.as_str() {
                                    extracted_content.push_str(&format!("Tag {}: {}\n", k, val_str));
                                }
                            }
                        }
                    }

                    if let Some(streams) = json.get("streams").and_then(|s| s.as_array()) {
                        let mut v_codecs = Vec::new();
                        let mut a_codecs = Vec::new();
                        let mut resolutions = Vec::new();

                        for stream in streams {
                            if let Some(codec_type) = stream.get("codec_type").and_then(|c| c.as_str()) {
                                if let Some(codec_name) = stream.get("codec_name").and_then(|c| c.as_str()) {
                                    match codec_type {
                                        "video" => {
                                            v_codecs.push(codec_name.to_string());
                                            let width = stream.get("width").and_then(|w| w.as_i64()).unwrap_or(0);
                                            let height = stream.get("height").and_then(|h| h.as_i64()).unwrap_or(0);
                                            if width > 0 && height > 0 {
                                                resolutions.push(format!("{}x{}", width, height));
                                            }
                                        },
                                        "audio" => a_codecs.push(codec_name.to_string()),
                                        _ => {}
                                    }
                                }
                            }
                        }
                        
                        if !v_codecs.is_empty() { extracted_content.push_str(&format!("Video Codecs: {}\n", v_codecs.join(", "))); }
                        if !a_codecs.is_empty() { extracted_content.push_str(&format!("Audio Codecs: {}\n", a_codecs.join(", "))); }
                        if !resolutions.is_empty() { extracted_content.push_str(&format!("Resolutions: {}\n", resolutions.join(", "))); }
                    }
                    extracted_content.push_str("----------------------\n\n");
                }
            }
        }

        // 2. Extract Embedded Subtitles/Dialogue via FFmpeg
        // We target the first subtitle stream (0:s:0) and dump it to stdout as plain text.
        if let Ok(ffmpeg_output) = Command::new("ffmpeg")
            .arg("-v")
            .arg("quiet")
            .arg("-i")
            .arg(&path_str)
            .arg("-map")
            .arg("0:s:0")
            .arg("-c:s")
            .arg("text")
            .arg("-f")
            .arg("srt")
            .arg("-")
            .output()
        {
            if ffmpeg_output.status.success() {
                let subtitles = String::from_utf8_lossy(&ffmpeg_output.stdout);
                if !subtitles.trim().is_empty() {
                    extracted_content.push_str("--- EMBEDDED DIALOGUE / SUBTITLES ---\n");
                    
                    // Strip out SRT timestamps and numeric identifiers to save vector space
                    for line in subtitles.lines() {
                        let trimmed = line.trim();
                        if !trimmed.is_empty() && !trimmed.contains("-->") && !trimmed.chars().all(|c| c.is_ascii_digit()) {
                            extracted_content.push_str(trimmed);
                            extracted_content.push(' ');
                        }
                    }
                    extracted_content.push_str("\n-------------------------------------\n");
                }
            }
        }

        if extracted_content.trim().is_empty() {
            Err("Failed to extract any metadata or subtitles from the video container.".to_string())
        } else {
            Ok(extracted_content)
        }
    }
}