// src/engine/llm/strategy_script.rs
use std::sync::Arc;
use std::sync::atomic::AtomicBool;
use super::{LlmStrategy, LlmCore};

pub struct ScriptCompilerStrategy;

impl LlmStrategy for ScriptCompilerStrategy {
    type Input = (String, Vec<String>);
    type Output = String;

    fn execute(&self, core: &LlmCore, input: Self::Input, is_cancelled: Arc<AtomicBool>) -> Self::Output {
        let (query, schema_keys) = input;
        
        let schema_str = if schema_keys.is_empty() {
            "No specific metadata fields available.".to_string()
        } else {
            schema_keys.join(", ")
        };

        let prompt = format!(
            "<|im_start|>system\nYou are a query routing compiler. Map user queries to a single boolean Rhai (Rust) expression. Use the provided Standard Library functions. Output ONLY the raw script code.<|im_end|>\n\
            <|im_start|>user\n\
            Available metadata fields: [{}]\n\
            \n\
            STANDARD LIBRARY FUNCTIONS:\n\
            - `metadata`: A map of strings (e.g., metadata.filetype)\n\
            - `text`: The document content snippet.\n\
            - `title`: The document filename/title.\n\
            - `regex_match(string, pattern)`: Executes a fast regex search.\n\
            - `regex_extract(string, pattern)`: Extracts the first regex match as a string.\n\
            - `in_list(string, list)`: Checks if a string is in a comma-separated list.\n\
            - `days_ago(float)`: Returns the UNIX timestamp for N days ago.\n\
            - `parse_float(string)`: Safely converts a string to a number.\n\
            - `contains_ignore_case(string, search)`: Case-insensitive substring match.\n\
            \n\
            Example Query: \"Receipts over 500\"\n\
            Example Output:\n\
            let is_receipt = contains_ignore_case(text, \"receipt\");\n\
            let amount_str = regex_extract(text, \"(?i)(?:total|amount|usd|\\\\$|cost)[^\\\\d]*(\\\\d+\\\\.?\\\\d*)\");\n\
            let amount_val = parse_float(amount_str);\n\
            is_receipt && amount_val > 500.0\n\
            \n\
            Example Query: \"Research papers cited more than 50 times\"\n\
            Example Output:\n\
            let is_paper = contains_ignore_case(text, \"research\") || contains_ignore_case(text, \"abstract\");\n\
            let cited_str = regex_extract(text, \"(?i)(?:cited|citations|times|index)[^\\\\d]*(\\\\d+\\\\.?\\\\d*)\");\n\
            let cited_val = parse_float(cited_str);\n\
            is_paper && cited_val > 50.0\n\
            \n\
            Example Query: \"MP3 songs longer than 3 minutes\"\n\
            Example Output:\n\
            let is_mp3 = metadata.filetype == \"mp3\" || contains_ignore_case(text, \"song\");\n\
            let length_str = regex_extract(text, \"(?i)(?:length|duration|minutes|time)[^\\\\d]*(\\\\d+\\\\.?\\\\d*)\");\n\
            let length_val = parse_float(length_str);\n\
            is_mp3 && length_val > 3.0\n\
            \n\
            CRITICAL RULES:\n\
            1. Output ONLY valid Rhai script. No markdown formatting, no ```rhai blocks, no explanations.\n\
            2. The script MUST implicitly return a boolean value at the end.\n\
            3. DEFAULT TO AND: ALWAYS use `&&` (AND) to combine conditions unless the user's prompt explicitly uses the word \"or\".\n\
            4. USE QUERY VALUES: You MUST extract the EXACT numbers and terms requested in the user's prompt. Do NOT blindly copy numbers from the examples.\n\
            5. ROBUST DYNAMIC EXTRACTION: When extracting numbers, you MUST dynamically generate context-relevant keywords for the non-capturing group based on the user's query domain. Pattern: `(?i)(?:<keyword1>|<keyword2>|<keyword3>)[^\\\\d]*(\\\\d+\\\\.?\\\\d*)`. This skips unpredictable formatting (newlines, symbols) between the keyword and the target number.\n\
            6. TRANSLATE LOGIC: NEVER use quantitative words like 'more', 'less', 'under', 'over' inside a literal regex pattern. You MUST extract the targeted number using `parse_float(regex_extract(...))` and then apply mathematical operators (>, <, >=, <=) inside the Rhai script.\n\
            \n\
            Query: \"{}\"<|im_end|>\n\
            <|im_start|>assistant\n",
            schema_str, query
        );

        // Lower token limit enforces concise functional chaining instead of sprawling procedural logic
        let response = core.generate_text("SCRIPT_COMPILER_STRATEGY", &prompt, 200, is_cancelled);
        
        let mut clean_resp = response.trim().to_string();
        if clean_resp.starts_with("```rhai") {
            clean_resp = clean_resp.trim_start_matches("```rhai").trim().to_string();
        } else if clean_resp.starts_with("```") {
            clean_resp = clean_resp.trim_start_matches("```").trim().to_string();
        }
        if clean_resp.ends_with("```") {
            clean_resp = clean_resp.trim_end_matches("```").trim().to_string();
        }
        
        clean_resp
    }
}