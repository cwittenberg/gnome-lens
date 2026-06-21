// src/engine/model_manager.rs
use std::env;
use std::fs;
use std::io::{Read, Write};
use std::path::Path;
use std::process::{Command, Stdio};

use serde_json::{json, Value};

pub struct ModelManager;

impl ModelManager {
    /// Bootstraps the application configuration and handles default deployment.
    ///
    /// Existing user configs are preserved:
    /// - Existing active_model is kept if it still points to a known model.
    /// - Existing model entries are not overwritten.
    /// - Missing default models are added automatically.
    /// - Missing fields inside existing default model entries are filled in.
    pub fn setup_model_config() -> Value {
        let home = env::var("HOME").expect("HOME environment variable must be set");
        let config_dir = format!("{}/.config/gnome-lens", home);
        let models_dir = format!("{}/.local/share/gnome-lens/models", home);

        fs::create_dir_all(&config_dir).expect("Failed to create config directory");
        fs::create_dir_all(&models_dir).expect("Failed to create models directory");

        let config_path = format!("{}/models.json", config_dir);
        let default_json = Self::default_model_config();

        if !Path::new(&config_path).exists() {
            fs::write(
                &config_path,
                serde_json::to_string_pretty(&default_json).expect("Failed to serialize default models.json"),
            )
            .expect("Failed to write default models.json");

            return default_json;
        }

        let parsed_config = match fs::read_to_string(&config_path) {
            Ok(content) => serde_json::from_str::<Value>(&content).unwrap_or_else(|_| default_json.clone()),
            Err(_) => default_json.clone(),
        };

        let (merged_config, changed) = Self::merge_with_default_model_config(parsed_config, &default_json);

        if changed {
            fs::write(
                &config_path,
                serde_json::to_string_pretty(&merged_config).expect("Failed to serialize merged models.json"),
            )
            .expect("Failed to write merged models.json");
        }

        merged_config
    }

    /// Resolves the absolute path and URL of the currently active model.
    pub fn get_active_model_path_and_url() -> (String, String) {
        let parsed_config = Self::setup_model_config();
        let fallback_config = Self::default_model_config();

        let active_key = parsed_config["active_model"]
            .as_str()
            .or_else(|| fallback_config["active_model"].as_str())
            .unwrap_or("qwen-2.5-3b");

        let model_obj = parsed_config["models"]
            .get(active_key)
            .or_else(|| fallback_config["models"].get(active_key))
            .or_else(|| fallback_config["models"].get("qwen-2.5-3b"))
            .expect("Default model configuration is missing qwen-2.5-3b");

        let filename = model_obj["filename"]
            .as_str()
            .unwrap_or("qwen2.5-3b-instruct-q4_k_m.gguf");

        let url = model_obj["url"]
            .as_str()
            .unwrap_or("https://huggingface.co/Qwen/Qwen2.5-3B-Instruct-GGUF/resolve/main/qwen2.5-3b-instruct-q4_k_m.gguf");

        let home = env::var("HOME").expect("HOME environment variable must be set");
        let model_path = format!("{}/.local/share/gnome-lens/models/{}", home, filename);

        (model_path, url.to_string())
    }

    /// Validates the model exists on disk, falling back to a blocking sync download.
    pub fn ensure_model_available(model_path: &str, url: &str) {
        if Self::model_file_exists(model_path) {
            return;
        }

        println!("\n=======================================================");
        println!("Local AI Model not found at: {}", model_path);
        println!("Downloading model from: {}", url);
        println!("This may take several minutes depending on your connection.");
        println!("=======================================================\n");

        if let Err(err) = Self::download_file_blocking(model_path, url) {
            panic!("{}", err);
        }
    }

    /// Dynamic async-like downloader that parses cURL output and pipes it to the GNOME UI socket.
    pub fn download_model_if_needed<F>(
        model_id: &str,
        send_chunk: &mut F,
    ) -> Result<String, String>
    where
        F: FnMut(String),
    {
        let parsed = Self::setup_model_config();

        let model_obj = parsed["models"]
            .get(model_id)
            .ok_or_else(|| format!("Model ID not found in configuration: {}", model_id))?;

        let filename = model_obj["filename"]
            .as_str()
            .ok_or_else(|| format!("Model '{}' is missing required field: filename", model_id))?;

        let url = model_obj["url"]
            .as_str()
            .ok_or_else(|| format!("Model '{}' is missing required field: url", model_id))?;

        let home = env::var("HOME").map_err(|_| "HOME environment variable must be set".to_string())?;
        let model_path = format!("{}/.local/share/gnome-lens/models/{}", home, filename);

        if Self::model_file_exists(&model_path) {
            return Ok(model_path);
        }

        if let Some(parent) = Path::new(&model_path).parent() {
            fs::create_dir_all(parent).map_err(|e| format!("Failed to create model directory: {}", e))?;
        }

        let temp_model_path = format!("{}.download", model_path);
        let _ = fs::remove_file(&temp_model_path);

        send_chunk(json!({
            "status": "processing",
            "message": "Connecting to model repository..."
        }).to_string());

        let mut child = Command::new("curl")
            .arg("-L")
            .arg("--fail")
            .arg("-#")
            .arg("-o")
            .arg(&temp_model_path)
            .arg(url)
            .stderr(Stdio::piped())
            .spawn()
            .map_err(|e| format!("Failed to start curl: {}", e))?;

        if let Some(stderr) = child.stderr.take() {
            let mut last_reported = -1;
            let mut current_line = String::new();

            for byte in stderr.bytes() {
                let b = match byte {
                    Ok(value) => value,
                    Err(_) => continue,
                };

                print!("{}", b as char);
                let _ = std::io::stdout().flush();

                if b == b'\r' || b == b'\n' {
                    if let Some(percent) = Self::parse_curl_progress_percent(&current_line) {
                        if percent > last_reported && percent % 2 == 0 {
                            send_chunk(json!({
                                "status": "processing",
                                "message": format!("Downloading model ({}%)...", percent)
                            }).to_string());

                            last_reported = percent;
                        }
                    }

                    current_line.clear();
                } else {
                    current_line.push(b as char);
                }
            }

            if let Some(percent) = Self::parse_curl_progress_percent(&current_line) {
                if percent > last_reported {
                    send_chunk(json!({
                        "status": "processing",
                        "message": format!("Downloading model ({}%)...", percent)
                    }).to_string());
                }
            }
        }

        let status = child
            .wait()
            .map_err(|_| "Download process failed to wait".to_string())?;

        if !status.success() {
            let _ = fs::remove_file(&temp_model_path);
            return Err("Download failed. Check internet connection or model URL.".to_string());
        }

        fs::rename(&temp_model_path, &model_path)
            .map_err(|e| {
                let _ = fs::remove_file(&temp_model_path);
                format!("Failed to finalize downloaded model: {}", e)
            })?;

        send_chunk(json!({
            "status": "processing",
            "message": "Model download completed."
        }).to_string());

        Ok(model_path)
    }

    /// Persists the active model selection to the config block.
    pub fn set_active_model(model_id: &str) -> Result<(), String> {
        let home = env::var("HOME").map_err(|_| "HOME environment variable must be set".to_string())?;
        let config_path = format!("{}/.config/gnome-lens/models.json", home);

        let mut parsed = Self::setup_model_config();

        if parsed["models"].get(model_id).is_none() {
            return Err(format!("Model ID not found in configuration: {}", model_id));
        }

        parsed["active_model"] = json!(model_id);

        fs::write(
            &config_path,
            serde_json::to_string_pretty(&parsed).map_err(|_| "Failed to serialize models.json".to_string())?,
        )
        .map_err(|_| "Failed to write updated models.json".to_string())?;

        Ok(())
    }

    fn default_model_config() -> Value {
        json!({
            "active_model": "qwen-2.5-3b",
            "models": {
                "llama-3.1-8b": {
                    "name": "Llama 3.1 (8B)",
                    "filename": "Meta-Llama-3.1-8B-Instruct-Q4_K_M.gguf",
                    "url": "https://huggingface.co/bartowski/Meta-Llama-3.1-8B-Instruct-GGUF/resolve/main/Meta-Llama-3.1-8B-Instruct-Q4_K_M.gguf",
                    "size_gb": 4.9,
                    "ram_required_gb": 6.5,
                    "parameters": "8.0B",
                    "context_tokens": 131072,
                    "category": "general",
                    "description": "Meta's strong general-purpose model. Good for RAG and broad assistant use, but Qwen is usually better for multilingual and coding use."
                },
                "qwen-2.5-3b": {
                    "name": "Qwen 2.5 (3B)",
                    "filename": "qwen2.5-3b-instruct-q4_k_m.gguf",
                    "url": "https://huggingface.co/Qwen/Qwen2.5-3B-Instruct-GGUF/resolve/main/qwen2.5-3b-instruct-q4_k_m.gguf",
                    "size_gb": 1.9,
                    "ram_required_gb": 2.8,
                    "parameters": "3.0B",
                    "context_tokens": 32768,
                    "category": "recommended-default",
                    "description": "Ultra-lightweight Qwen model. Excellent default for quick local responses, translation, summarization, and reliable Vulkan compatibility."
                },
                "qwen-2.5-7b": {
                    "name": "Qwen 2.5 (7B)",
                    "filename": "qwen2.5-7b-instruct-q4_k_m.gguf",
                    "url": "https://huggingface.co/Qwen/Qwen2.5-7B-Instruct-GGUF/resolve/main/qwen2.5-7b-instruct-q4_k_m.gguf",
                    "size_gb": 4.7,
                    "ram_required_gb": 6.5,
                    "parameters": "7.0B",
                    "context_tokens": 32768,
                    "category": "balanced",
                    "description": "Strong multilingual general-purpose model. Better quality than 3B while still realistic on small desktops."
                },
                "qwen2.5-coder-7b-q4-k-m": {
                    "name": "Qwen 2.5 Coder (7B)",
                    "filename": "Qwen2.5-Coder-7B-Instruct-Q4_K_M.gguf",
                    "url": "https://huggingface.co/bartowski/Qwen2.5-Coder-7B-Instruct-GGUF/resolve/main/Qwen2.5-Coder-7B-Instruct-Q4_K_M.gguf",
                    "size_gb": 4.68,
                    "ram_required_gb": 7.0,
                    "parameters": "7.0B",
                    "context_tokens": 32768,
                    "category": "coding",
                    "description": "Fast coding-specialized model. Realistic default choice for code explanation, snippets, refactoring, and developer workflows."
                },
                "qwen2.5-coder-14b-q4-k-m": {
                    "name": "Qwen 2.5 Coder (14B)",
                    "filename": "qwen2.5-coder-14b-instruct-q4_k_m.gguf",
                    "url": "https://huggingface.co/Qwen/Qwen2.5-Coder-14B-Instruct-GGUF/resolve/main/qwen2.5-coder-14b-instruct-q4_k_m.gguf",
                    "size_gb": 8.9,
                    "ram_required_gb": 12.0,
                    "parameters": "14.7B",
                    "context_tokens": 32768,
                    "category": "coding",
                    "description": "Serious local coding model. Slower than 7B, but much stronger for code reasoning and multi-step fixes."
                },
                "qwen3-8b-q4-k-m": {
                    "name": "Qwen 3 (8B)",
                    "filename": "Qwen3-8B-Q4_K_M.gguf",
                    "url": "https://huggingface.co/Qwen/Qwen3-8B-GGUF/resolve/main/Qwen3-8B-Q4_K_M.gguf",
                    "size_gb": 4.7,
                    "ram_required_gb": 7.0,
                    "parameters": "8.2B",
                    "context_tokens": 32768,
                    "category": "general",
                    "description": "Strong reasoning, multilingual support, and good speed at Q4_K_M."
                },
                "qwen3-14b-q4-k-m": {
                    "name": "Qwen 3 (14B)",
                    "filename": "Qwen3-14B-Q4_K_M.gguf",
                    "url": "https://huggingface.co/bartowski/Qwen_Qwen3-14B-GGUF/resolve/main/Qwen3-14B-Q4_K_M.gguf",
                    "size_gb": 9.0,
                    "ram_required_gb": 13.0,
                    "parameters": "14.8B",
                    "context_tokens": 32768,
                    "category": "large-general",
                    "description": "Higher-quality Qwen3 option that still fits comfortably in 32 GB RAM. Good for reasoning when speed matters less."
                },
                "qwen3-coder-30b-a3b-ud-q4-k-xl": {
                    "name": "Qwen 3 Coder 30B-A3B",
                    "filename": "Qwen3-Coder-30B-A3B-Instruct-UD-Q4_K_XL.gguf",
                    "url": "https://huggingface.co/unsloth/Qwen3-Coder-30B-A3B-Instruct-GGUF/resolve/main/Qwen3-Coder-30B-A3B-Instruct-UD-Q4_K_XL.gguf",
                    "size_gb": 17.7,
                    "ram_required_gb": 24.0,
                    "parameters": "30.5B total / 3.3B active",
                    "context_tokens": 32768,
                    "category": "heavy-coding",
                    "recommended": false,
                    "description": "Cool high-end local coding model for your 32 GB machine. Realistic, but heavy; expect slower startup and inference."
                },
                "devstral-small-2507-q4-k-m": {
                    "name": "Devstral Small 2507",
                    "filename": "Devstral-Small-2507-Q4_K_M.gguf",
                    "url": "https://huggingface.co/mistralai/Devstral-Small-2507_gguf/resolve/main/Devstral-Small-2507-Q4_K_M.gguf",
                    "size_gb": 14.33,
                    "ram_required_gb": 22.0,
                    "parameters": "24B",
                    "context_tokens": 131072,
                    "category": "agentic-coding",
                    "recommended": false,
                    "description": "Agentic software-engineering model. Fits in 32 GB RAM at Q4_K_M, but it is heavy and better suited to long coding-agent sessions."
                }
            }
        })
    }

    fn merge_with_default_model_config(mut current: Value, defaults: &Value) -> (Value, bool) {
        let mut changed = false;

        if !current.is_object() {
            return (defaults.clone(), true);
        }

        if current.get("active_model").and_then(Value::as_str).is_none() {
            current["active_model"] = defaults["active_model"].clone();
            changed = true;
        }

        if current.get("models").and_then(Value::as_object).is_none() {
            current["models"] = defaults["models"].clone();
            changed = true;
        }

        if let (Some(current_models), Some(default_models)) = (
            current.get_mut("models").and_then(Value::as_object_mut),
            defaults.get("models").and_then(Value::as_object),
        ) {
            for (model_id, default_model) in default_models {
                if !current_models.contains_key(model_id) {
                    current_models.insert(model_id.clone(), default_model.clone());
                    changed = true;
                    continue;
                }

                if let Some(existing_model) = current_models.get_mut(model_id) {
                    if let (Some(existing_fields), Some(default_fields)) = (
                        existing_model.as_object_mut(),
                        default_model.as_object(),
                    ) {
                        for (field_name, default_field_value) in default_fields {
                            if !existing_fields.contains_key(field_name) {
                                existing_fields.insert(field_name.clone(), default_field_value.clone());
                                changed = true;
                            }
                        }
                    }
                }
            }
        }

        let active_model_id = current["active_model"]
            .as_str()
            .unwrap_or("qwen-2.5-3b")
            .to_string();

        let active_model_exists = current["models"].get(&active_model_id).is_some();

        if !active_model_exists {
            current["active_model"] = defaults["active_model"].clone();
            changed = true;
        }

        (current, changed)
    }

    fn model_file_exists(model_path: &str) -> bool {
        fs::metadata(model_path)
            .map(|metadata| metadata.is_file() && metadata.len() > 0)
            .unwrap_or(false)
    }

    fn download_file_blocking(model_path: &str, url: &str) -> Result<(), String> {
        if let Some(parent) = Path::new(model_path).parent() {
            fs::create_dir_all(parent).map_err(|e| format!("Failed to create model directory: {}", e))?;
        }

        let temp_model_path = format!("{}.download", model_path);
        let _ = fs::remove_file(&temp_model_path);

        let status = Command::new("curl")
            .arg("-L")
            .arg("--fail")
            .arg("-#")
            .arg("-o")
            .arg(&temp_model_path)
            .arg(url)
            .status()
            .map_err(|e| format!("Failed to execute curl to download the model: {}", e))?;

        if !status.success() {
            let _ = fs::remove_file(&temp_model_path);
            return Err("Failed to download the model. Please check your internet connection or model URL.".to_string());
        }

        fs::rename(&temp_model_path, model_path)
            .map_err(|e| {
                let _ = fs::remove_file(&temp_model_path);
                format!("Failed to finalize downloaded model: {}", e)
            })?;

        Ok(())
    }

    fn parse_curl_progress_percent(line: &str) -> Option<i32> {
        for token in line.split_whitespace().rev() {
            let cleaned = token
                .trim()
                .trim_end_matches('%')
                .trim_matches('#')
                .trim_matches('-')
                .trim_matches('=');

            if cleaned.is_empty() {
                continue;
            }

            if let Ok(value) = cleaned.parse::<f32>() {
                if (0.0..=100.0).contains(&value) {
                    return Some(value.round() as i32);
                }
            }
        }

        None
    }
}