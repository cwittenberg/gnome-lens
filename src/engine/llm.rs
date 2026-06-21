// src/engine/llm.rs
use std::sync::{Arc, Mutex};
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};
use std::io::Write;

use llama_cpp_2::llama_backend::LlamaBackend;
use llama_cpp_2::context::params::LlamaContextParams;
use llama_cpp_2::model::LlamaModel;
use llama_cpp_2::model::params::LlamaModelParams;

use crate::domain::{SearchQuery, SearchResult};
use crate::engine::model_manager::ModelManager;
use crate::engine::HardwareManager;

#[derive(PartialEq)]
pub enum LlmIntent {
    Skip,
    RefineSearch,
    SynthesizeAnswer,
    FilterResults,
}

pub struct LlmEngine {
    pub backend: LlamaBackend,
    pub model: LlamaModel,
}

pub struct LlmService {
    engine: Arc<Mutex<LlmEngine>>,
}

impl LlmService {
    pub fn new() -> Self {
        println!("Initializing llama.cpp Backend...");
        
        let backend = LlamaBackend::init().expect("Failed to initialize C++ backend");
        let (model_path, model_url) = ModelManager::get_active_model_path_and_url();
        
        ModelManager::ensure_model_available(&model_path, &model_url);

        let n_gpu = HardwareManager::get_optimal_gpu_layers();
        
        println!("[LLM] Note: If 'token_embd.weight' is mapped to the CPU in the following logs, this is EXPECTED.");
        println!("[LLM] The embedding lookup table stays in system RAM for fast O(1) lookups, while all compute layers offload to the GPU.");
        
        let model_params = LlamaModelParams::default().with_n_gpu_layers(n_gpu);
        
        let model = LlamaModel::load_from_file(&backend, &model_path, &model_params)
            .unwrap_or_else(|_| panic!("Failed to load GGUF model from {}.", model_path));

        Self {
            engine: Arc::new(Mutex::new(LlmEngine { backend, model })),
        }
    }

    pub fn switch_model<F>(&self, model_id: &str, send_chunk: &mut F, is_cancelled: Arc<AtomicBool>) -> Result<(), String> 
    where F: FnMut(String) 
    {
        let model_path = ModelManager::download_model_if_needed(model_id, send_chunk, is_cancelled)?;

        send_chunk(serde_json::json!({"status": "processing", "message": "Loading model into memory..."}).to_string());
        
        let mut engine_guard = self.engine.lock().unwrap();
        
        let n_gpu = HardwareManager::get_optimal_gpu_layers();
        
        println!("[LLM] Note: If 'token_embd.weight' is mapped to the CPU in the following logs, this is EXPECTED.");
        println!("[LLM] The embedding lookup table stays in system RAM for fast O(1) lookups, while all compute layers offload to the GPU.");
        
        let new_model = LlamaModel::load_from_file(&engine_guard.backend, &model_path, &LlamaModelParams::default().with_n_gpu_layers(n_gpu))
            .map_err(|_| "Corrupted model file or failed to load into RAM")?;
        
        ModelManager::set_active_model(model_id)?;
        
        engine_guard.model = new_model;

        Ok(())
    }

    fn generate_text(&self, prompt: &str, max_tokens: usize, is_cancelled: Arc<AtomicBool>) -> String {
        println!("\n=======================================================================================");
        println!("[DEBUG] RAW TEXT/PROMPT FED TO THE LLM:");
        println!("---------------------------------------------------------------------------------------");
        println!("{}", prompt);
        println!("=======================================================================================\n");

        let engine = match self.engine.lock() {
            Ok(guard) => guard,
            Err(poisoned) => poisoned.into_inner(),
        };
        
        let n_ctx_limit: u32 = 4096;
        let n_batch_limit: u32 = 512;
        
        let available_threads = std::thread::available_parallelism()
            .map(|n| n.get() as i32)
            .unwrap_or(4);
        let optimal_threads = available_threads.min(8);
        
        let ctx_params = LlamaContextParams::default()
            .with_n_ctx(std::num::NonZeroU32::new(n_ctx_limit))
            .with_n_batch(n_batch_limit)
            .with_n_threads(optimal_threads)
            .with_n_threads_batch(optimal_threads);
            
        let mut ctx = match engine.model.new_context(&engine.backend, ctx_params) {
            Ok(c) => c,
            Err(_) => return String::new(),
        };

        let mut tokens_list = engine.model.str_to_token(prompt, llama_cpp_2::model::AddBos::Always)
            .unwrap_or_default();

        let safe_max = if max_tokens > n_ctx_limit as usize { (n_ctx_limit / 2) as usize } else { max_tokens };
        let max_prompt_len = (n_ctx_limit as usize).saturating_sub(safe_max).saturating_sub(10);
        
        if tokens_list.len() > max_prompt_len {
            let excess = tokens_list.len() - max_prompt_len;
            
            let start_drain = 500.min(tokens_list.len() / 2);
            let end_drain = (start_drain + excess).min(tokens_list.len().saturating_sub(20));

            if start_drain < end_drain {
                tokens_list.drain(start_drain..end_drain);
            } else {
                tokens_list.truncate(max_prompt_len);
            }
        }

        if tokens_list.is_empty() { 
            return String::new(); 
        }

        println!("[LLM] Ingesting prompt ({} tokens) on {} threads...", tokens_list.len(), optimal_threads);
        
        let mut batch = llama_cpp_2::llama_batch::LlamaBatch::new(n_batch_limit as usize, 1);
        let mut n_cur = 0;

        for chunk in tokens_list.chunks(n_batch_limit as usize) {
            if is_cancelled.load(Ordering::Relaxed) {
                println!("\n[LLM] Request cancelled by client during ingestion.");
                return String::new();
            }

            batch.clear();
            let is_last_chunk = n_cur + chunk.len() == tokens_list.len();
            
            for (i, &token) in chunk.iter().enumerate() {
                let is_last_token = is_last_chunk && (i == chunk.len() - 1);
                if batch.add(token, (n_cur + i) as i32, &[0], is_last_token).is_err() {
                    return String::new();
                }
            }
            
            if ctx.decode(&mut batch).is_err() {
                return String::new();
            }
            n_cur += chunk.len();
            print!(".");
            let _ = std::io::stdout().flush();
        }
        
        println!(" [Done]");
        print!("[LLM] Generating: ");
        let _ = std::io::stdout().flush();

        let mut output = String::new();
        let mut decoder = encoding_rs::UTF_8.new_decoder();
        let absolute_max = (tokens_list.len() + max_tokens).min(n_ctx_limit as usize);

        while n_cur < absolute_max {
            if is_cancelled.load(Ordering::Relaxed) {
                println!("\n[LLM] Request cancelled by client during generation.");
                break;
            }

            let candidates = ctx.candidates_ith(batch.n_tokens() - 1);
            
            let mut best_token = engine.model.token_eos();
            let mut max_logit = f32::NEG_INFINITY;

            for cand in candidates {
                if cand.logit() > max_logit {
                    max_logit = cand.logit();
                    best_token = cand.id();
                }
            }

            if best_token == engine.model.token_eos() {
                break;
            }

            let token_str = engine.model.token_to_piece(best_token, &mut decoder, true, None)
                .unwrap_or_default();
                
            output.push_str(&token_str);
            
            print!("{}", token_str);
            let _ = std::io::stdout().flush();

            // Exhaustive stop condition catching all variants of End-of-Generation
            if output.contains("<|end|>") 
                || output.contains("<|user|>") 
                || output.contains("<|assistant|>") 
                || output.contains("<|eot_id|>") 
                || output.contains("<|im_end|>") 
                || output.contains("<|im_start|>")
                || output.contains("<|endoftext|>")
                || output.contains("</s>") 
            {
                break;
            }

            batch.clear();
            if batch.add(best_token, n_cur as i32, &[0], true).is_err() {
                break;
            }
            
            if ctx.decode(&mut batch).is_err() {
                break;
            }
            n_cur += 1;
        }

        println!("\n[LLM] Request complete.");
        output
    }

    pub fn determine_intent(&self, query: &str, explicit_synthesis: bool, is_cancelled: Arc<AtomicBool>) -> LlmIntent {
        if explicit_synthesis { return LlmIntent::SynthesizeAnswer; }

        let lower = query.to_lowercase();
        let words: Vec<&str> = lower.split(|c: char| !c.is_alphanumeric()).collect();
        
        let synthesis_triggers = [
            "what", "how", "why", "who", "when", "where", "which", "explain", "summarize",
            "wat", "hoe", "waarom", "wie", "wanneer", 
            "warum", "wer", "wann", 
            "que", "como", "porque", "quien", "donde", "cual", "explique", "resuma"
        ];
        
        let filter_triggers = [
            "less", "greater", "under", "over", "below", "above", "only", "larger", "smaller", "exactly",
            "minder", "meer", 
            "unter", "über", 
            "menos", "mayor", "debajo", "encima"
        ];
        
        let time_triggers = [
            "ago", "last", "past", "days", "weeks", "months", "years", "before", "after", "yesterday", "today",
            "geleden", "vorige", "laatste", "gisteren", "vandaag",
            "vor", "letzte", "gestern", "heute",
            "hace", "pasado", "dias", "semanas", "meses", "años", "ayer", "hoy"
        ];

        let mut has_trigger = false;
        
        if synthesis_triggers.iter().any(|&w| words.contains(&w)) { has_trigger = true; }
        if filter_triggers.iter().any(|&w| words.contains(&w)) { has_trigger = true; }
        if time_triggers.iter().any(|&w| words.contains(&w)) { has_trigger = true; }

        if !has_trigger {
            return LlmIntent::Skip;
        }

        let prompt = format!(
            "<|im_start|>system\nYou are a strict routing API. Output ONLY a single digit.<|im_end|>\n\
            <|im_start|>user\n\
            Classify the user's search intent into ONE digit:\n\
            1: SKIP (Keywords only, e.g., 'budget report')\n\
            2: REFINE_TIME (Time filters, e.g., 'last year')\n\
            3: FILTER_VALUE (Math filters, e.g., 'under 100 usd')\n\
            4: SYNTHESIZE (Questions, e.g., 'explain how')\n\n\
            CRITICAL RULES:\n\
            - If the query contains quantitative operators ('below', 'under', 'greater', 'less', 'above'), answer 3.\n\
            - Semantic descriptions like 'about X' or 'for Y' answer 1.\n\n\
            Query: \"{}\"<|im_end|>\n\
            <|im_start|>assistant\n\
            INTENT_DIGIT: ",
            query
        );

        let response = self.generate_text(&prompt, 5, is_cancelled).trim().to_string();
        let clean_response = response.replace("INTENT_DIGIT:", "");
        
        if clean_response.contains('2') { return LlmIntent::RefineSearch; }
        if clean_response.contains('3') { return LlmIntent::FilterResults; }
        if clean_response.contains('4') { return LlmIntent::SynthesizeAnswer; }
        LlmIntent::Skip
    }

    pub fn apply_temporal_heuristics(&self, query: &mut SearchQuery, is_cancelled: Arc<AtomicBool>) {
        let now = SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_secs();
        let lower_text = query.raw_text.to_lowercase();

        let prompt = format!(
            "<|im_start|>system\nYou are a strict data extraction tool. Output ONLY pipe-separated values.<|im_end|>\n\
            <|im_start|>user\n\
            Extract time constraints from the query. Current UNIX timestamp: {}.\n\
            Format: MIN_TS|MAX_TS|CLEAN_QUERY\n\
            Use 0 for missing timestamps. Do not output anything else.\n\
            Example: 0|1690000000|invoices\n\n\
            Query: \"{}\"<|im_end|>\n\
            <|im_start|>assistant\n\
            TEMPORAL_DATA: ",
            now, lower_text
        );

        let response = self.generate_text(&prompt, 50, is_cancelled);
        let clean_response = response.replace("TEMPORAL_DATA:", "");
        let parts: Vec<&str> = clean_response.split('|').collect();
        
        if parts.len() >= 3 {
            if let Ok(min) = parts[0].trim().parse::<u64>() { 
                if min > 0 { query.min_timestamp = Some(min); } 
            }
            if let Ok(max) = parts[1].trim().parse::<u64>() { 
                if max > 0 { query.max_timestamp = Some(max); } 
            }
            query.raw_text = parts[2..].join("|").trim().to_string();
        }
    }

    fn extract_relevant_window(text: &str, condition: &str, window_chars: usize) -> String {
        let stop_words = [
            "what", "how", "why", "who", "when", "the", "and", "for", "with", "that", "this", "are", "you", "from", "does", "was", "is", "a", "an", "of", "in", "to",
            "que", "como", "por", "quien", "cuando", "el", "la", "los", "las", "y", "para", "con", "eso", "esto", "son", "tu", "desde", "hace", "era", "es", "un", "una", "de", "en",
            "wat", "hoe", "waarom", "wie", "wanneer", "de", "het", "en", "voor", "met", "dat", "dit", "zijn", "jij", "van", "doet", "was", "is", "een", "in", "naar"
        ];
        let lower_text = text.to_lowercase();
        
        if text.len() <= window_chars + 500 {
            return text.to_string();
        }

        let clean_cond: String = condition.to_lowercase().chars().filter(|c| c.is_alphanumeric() || *c == ' ').collect();

        let query_terms: Vec<&str> = clean_cond.split_whitespace()
            .filter(|t| t.len() > 2 && !stop_words.contains(t))
            .collect();

        if query_terms.is_empty() {
            let half = window_chars / 2;
            let head = text.chars().take(half).collect::<String>();
            let tail: String = text.chars().rev().take(half).collect::<Vec<char>>().into_iter().rev().collect();
            return format!("{}\n\n...[TRUNCATED]...\n\n{}", head, tail);
        }

        let mut positions = Vec::new();
        for term in &query_terms {
            let mut start = 0;
            while let Some(pos) = lower_text[start..].find(term) {
                let absolute_pos = start + pos;
                positions.push(absolute_pos);
                start = absolute_pos + term.len();
            }
        }

        if positions.is_empty() {
            let half = window_chars / 2;
            let head = text.chars().take(half).collect::<String>();
            let tail: String = text.chars().rev().take(half).collect::<Vec<char>>().into_iter().rev().collect();
            return format!("{}\n\n...[TRUNCATED]...\n\n{}", head, tail);
        }

        positions.sort_unstable();
        let mut best_byte_start = positions[0];
        let mut max_density = 0;
        
        let window_bytes = window_chars * 2; 

        for i in 0..positions.len() {
            let start_pos = positions[i];
            let end_pos = start_pos + window_bytes;
            let mut count = 0;

            for j in i..positions.len() {
                if positions[j] < end_pos {
                    count += 1;
                } else {
                    break;
                }
            }

            if count > max_density {
                max_density = count;
                best_byte_start = start_pos;
            }
        }

        let start_char_idx = text[..best_byte_start].chars().count();
        let safe_start = start_char_idx.saturating_sub(60);
        
        text.chars().skip(safe_start).take(window_chars).collect()
    }

    pub fn filter_with_llm(&self, condition: &str, candidates: Vec<SearchResult>, is_cancelled: Arc<AtomicBool>) -> Vec<SearchResult> {
        if candidates.is_empty() { return vec![]; }

        let mut all_processed = Vec::new();
        let max_batch_chars = 15_000;

        let mut chunks: Vec<Vec<SearchResult>> = Vec::new();
        let mut current_chunk = Vec::new();
        let mut current_chars = 0;

        for doc in candidates.into_iter() {
            let content = doc.full_context.as_deref().unwrap_or(&doc.snippet);
            let safe_content_len = Self::extract_relevant_window(content, condition, 800).len();
            let estimated_len = safe_content_len + 100; 

            if !current_chunk.is_empty() && current_chars + estimated_len > max_batch_chars {
                chunks.push(current_chunk);
                current_chunk = Vec::new();
                current_chars = 0;
            }
            current_chunk.push(doc);
            current_chars += estimated_len;
        }
        if !current_chunk.is_empty() {
            chunks.push(current_chunk);
        }

        for mut chunk in chunks {
            if is_cancelled.load(Ordering::Relaxed) { break; }

            let mut docs_block = String::new();
            for (i, doc) in chunk.iter().enumerate() {
                let is_shallow = doc.metadata.get("shallow_index").map(|v| v.as_str()) == Some("true");
                
                if is_shallow {
                    docs_block.push_str(&format!(
                        "--- START DOCUMENT ID: {} ---\nFILENAME: {}\nMETADATA: {:?}\n(CONTENT UNINDEXED. EVALUATE BY METADATA)\n--- END DOCUMENT ID: {} ---\n\n", 
                        i, doc.title, doc.metadata, i
                    ));
                } else {
                    let content = doc.full_context.as_deref().unwrap_or(&doc.snippet);
                    let safe_content = Self::extract_relevant_window(content, condition, 800); 
                    docs_block.push_str(&format!("--- START DOCUMENT ID: {} ---\n{}\n--- END DOCUMENT ID: {} ---\n\n", i, safe_content, i));
                }
            }

            let prompt = format!(
                "<|im_start|>system\nYou are a strict data extraction tool. Output ONLY a comma-separated list of integer IDs.<|im_end|>\n\
                <|im_start|>user\n\
                Evaluate which documents satisfy this condition: \"{}\".\n\
                CRITICAL INSTRUCTIONS:\n\
                1. Output ONLY a comma-separated list of integer IDs for the documents that pass the condition.\n\
                2. Do NOT output JSON, reasoning, markdown, or conversational filler.\n\
                3. If no documents pass, output exactly the word NONE.\n\
                Example Output: 0, 2, 4\n\n\
                DOCUMENTS:\n{}<|im_end|>\n\
                <|im_start|>assistant\n\
                MATCHING_IDS: ",
                condition, docs_block
            );

            // Limited generation tokens heavily to enforce brevity
            let response = self.generate_text(&prompt, 30, Arc::clone(&is_cancelled));
            
            let mut matched_indices = Vec::new();
            
            // Clean out the conversational prefix just in case the LLM repeats it
            let clean_response = response.replace("MATCHING_IDS:", "").replace("NONE", "");

            // Handle potential whitespace or commas from the raw text efficiently
            for token in clean_response.replace(',', " ").split_whitespace() {
                if let Ok(id) = token.parse::<usize>() {
                    matched_indices.push(id);
                }
            }

            for (i, doc) in chunk.iter_mut().enumerate() {
                let is_match = matched_indices.contains(&i);
                doc.ai_matched = Some(is_match);
                // Assign a generic reason locally rather than forcing the LLM to generate one for every file
                doc.ai_reasoning = Some(if is_match { "Condition = True".to_string() } else { "Condition = False".to_string() });
            }
            
            all_processed.extend(chunk.iter().cloned());
        }

        all_processed
    }

    pub fn generate_synthesis(&self, query: &str, mut context_docs: Vec<SearchResult>, is_cancelled: Arc<AtomicBool>) -> serde_json::Value {
        if context_docs.is_empty() {
            return serde_json::json!({
                "answer": "No documents found.",
                "reasoning": "Vector search returned empty.",
                "confidence_score": 0,
                "confidence_justification": "Missing context"
            });
        }

        context_docs.truncate(4);
        let mut context_block = String::new();
        
        for (i, doc) in context_docs.iter().enumerate() {
            let is_shallow = doc.metadata.get("shallow_index").map(|v| v.as_str()) == Some("true");
            
            if is_shallow {
                context_block.push_str(&format!(
                    "Source [{}] (SHALLOW FILE METADATA ONLY):\nFilename: {}\nPath: {:?}\nNote: The content of this file is currently unindexed and unreadable. You may inform the user that this file exists and could be relevant.\n\n", 
                    i + 1, doc.title, doc.filepath
                ));
            } else {
                let content = doc.full_context.as_deref().unwrap_or(&doc.snippet);
                let safe_content = Self::extract_relevant_window(content, query, 1500);
                context_block.push_str(&format!("Source [{}] ({}):\n{}\n\n", i + 1, doc.title, safe_content));
            }
        }

        let prompt = format!(
            "<|im_start|>system\nYou are an analytical AI. Answer using ONLY the provided context.<|im_end|>\n\
            <|im_start|>user\n\
            CRITICAL INSTRUCTIONS:\n\
            1. Format your response EXACTLY like this, with nothing else:\n\
            ANSWER: <final concise answer citing sources>\n\
            REASONING: <brief 1 sentence derivation>\n\
            2. Cite sources in the 'answer' (e.g., 'According to doc1...').\n\
            3. If a source is a 'SHALLOW FILE', you cannot see its content. Suggest opening it.\n\n\
            CONTEXT:\n{}\n\n\
            QUERY: {}<|im_end|>\n\
            <|im_start|>assistant\n\
            ANSWER: ",
            context_block, query
        );

        let response = self.generate_text(&prompt, 300, is_cancelled);
        
        // Re-construct the full string since we primed the generation with "ANSWER: "
        let full_response = format!("ANSWER: {}", response);
        
        let mut answer = String::new();
        let mut reasoning = String::new();

        if let Some(ans_idx) = full_response.find("ANSWER:") {
            if let Some(res_idx) = full_response.find("REASONING:") {
                if ans_idx < res_idx {
                    answer = full_response[ans_idx + 7..res_idx].trim().to_string();
                    reasoning = full_response[res_idx + 10..].trim().to_string();
                }
            } else {
                answer = full_response[ans_idx + 7..].trim().to_string();
            }
        }

        if answer.is_empty() { 
            answer = full_response; 
        }

        // Translate the efficient text response natively into the JSON structure your UI expects
        serde_json::json!({
            "answer": answer,
            "reasoning": reasoning,
            "confidence_score": 100,
            "confidence_justification": "Derived natively"
        })
    }
}