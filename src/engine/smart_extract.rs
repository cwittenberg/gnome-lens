use fancy_regex::Regex;
use serde::{Serialize, Deserialize};
use std::collections::HashSet;

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct SmartEntity {
    pub id: String,
    pub label: String,
    pub icon: String,
    pub value: String,
    #[serde(rename = "matchLabel")]
    pub match_label: String,
    pub confidence: f64,
    pub uri: Option<String>,
}

pub struct SmartExtractor {
    url_re: Regex,
    email_re: Regex,
    ipv4_re: Regex,
    ipv6_re: Regex,
    mac_re: Regex,
    iban_re: Regex,
    uuid_re: Regex,
    hex_color_re: Regex,
    date_re: Regex,
    phone_re: Regex,
    number_re: Regex,
}

impl SmartExtractor {
    pub fn new() -> Self {
        let hex = "[0-9a-fA-FSOILZGTQsoilzgtq]";

        let ipv4 = r"(?<![0-9.])(?:(?:25[0-5]|2[0-4][0-9]|[01]?[0-9][0-9]?)\s*\.\s*){3}(?:25[0-5]|2[0-4][0-9]|[01]?[0-9][0-9]?)(?:\s*/\s*(?:3[0-2]|[1-2]?[0-9]))?(?![0-9.])";
        
        let ipv6 = Self::build_ipv6_pattern(hex);

        let mac = format!(r"(?<!{}|[:\-])(?:{}{{2}}(?:\s*[:\-]\s*)){{5}}{}{{2}}(?!{}|[:\-])", hex, hex, hex, hex);

        let url = r"(?<![a-zA-Z0-9])https?://(?:www\.)?[-a-zA-Z0-9@:%._\+~#=]{1,256}\.[a-zA-Z0-9()]{1,6}\b(?:[-a-zA-Z0-9()@:%_\+.~#?&/=]*)";

        let email = r"(?<![a-zA-Z0-9._%+-])[a-zA-Z0-9._%+-]+@[a-zA-Z0-9.-]+\.[a-zA-Z]{2,}(?![a-zA-Z0-9])";

        let iban = r"(?<![A-Z0-9])[A-Z]{2}\d{2}(?:[ \-]?[A-Z0-9]){11,30}(?![A-Z0-9])";

        let uuid = r"(?<![0-9a-fA-F\-])[0-9a-fA-F]{8}-[0-9a-fA-F]{4}-[0-9a-fA-F]{4}-[0-9a-fA-F]{4}-[0-9a-fA-F]{12}(?![0-9a-fA-F\-])";

        let hex_color = format!(r"(?<![a-zA-Z0-9])#(?:{}{{6}}|{}{{3}})(?![a-zA-Z0-9])", hex, hex);

        let date = r"(?<!\d)(?:\d{4}[-/]\d{2}[-/]\d{2}|\d{2}[-/]\d{2}[-/]\d{4})(?!\d)";

        let phone = r"(?<!\d)(?:(?:\+|00)\d{1,3}[\s-]?)?(?:\(?[0-9]{1,4}\)?[\s-]?)?(?:\d[\s-]?){6,10}(?!\d)";

        let number = r"(?<![a-zA-Z0-9])\d(?:[\s\-._]?\d){4,}(?![a-zA-Z0-9])";

        Self {
            url_re: Regex::new(&format!("(?i){}", url)).unwrap(),
            email_re: Regex::new(&format!("(?i){}", email)).unwrap(),
            ipv4_re: Regex::new(ipv4).unwrap(), // No (?i) needed for IPv4
            ipv6_re: Regex::new(&format!("(?i){}", ipv6)).unwrap(),
            mac_re: Regex::new(&format!("(?i){}", mac)).unwrap(),
            iban_re: Regex::new(&format!("(?i){}", iban)).unwrap(),
            uuid_re: Regex::new(&format!("(?i){}", uuid)).unwrap(),
            hex_color_re: Regex::new(&format!("(?i){}", hex_color)).unwrap(),
            date_re: Regex::new(&format!("(?i){}", date)).unwrap(),
            phone_re: Regex::new(&format!("(?i){}", phone)).unwrap(),
            number_re: Regex::new(&format!("(?i){}", number)).unwrap(),
        }
    }

    fn build_ipv6_pattern(hex: &str) -> String {
        let groups = vec![
            format!(r"(?:{}{{1,4}}:){{7,7}}{}{{1,4}}", hex, hex),
            format!(r"(?:{}{{1,4}}:){{1,7}}:", hex),
            format!(r"(?:{}{{1,4}}:){{1,6}}:{}{{1,4}}", hex, hex),
            format!(r"(?:{}{{1,4}}:){{1,5}}(?::{}{{1,4}}){{1,2}}", hex, hex),
            format!(r"(?:{}{{1,4}}:){{1,4}}(?::{}{{1,4}}){{1,3}}", hex, hex),
            format!(r"(?:{}{{1,4}}:){{1,3}}(?::{}{{1,4}}){{1,4}}", hex, hex),
            format!(r"(?:{}{{1,4}}:){{1,2}}(?::{}{{1,4}}){{1,5}}", hex, hex),
            format!(r"{}{{1,4}}:(?:(?::{}{{1,4}}){{1,6}})", hex, hex),
            format!(r":(?:(?::{}{{1,4}}){{1,7}}|:)", hex), // Fixed unused argument
        ];
        let groups_joined = groups.join("|");
        let suffix = r"(?:%[a-zA-Z0-9_]+)?(?:/(?:12[0-8]|1[0-1][0-9]|[1-9]?[0-9]))?";
        let base = format!(r"(?:{}){}", groups_joined, suffix);

        let replaced1 = base.replace("(?:", "___NONCAP___");
        let replaced2 = replaced1.replace(":", r"\s*:\s*");
        let replaced3 = replaced2.replace("___NONCAP___", "(?:");
        let replaced4 = replaced3.replace("/", r"\s*/\s*");

        format!(r"(?<!{}|:)(?:{})(?!{}|:)", hex, replaced4, hex)
    }

    pub fn extract_entities(&self, text: &str) -> Vec<SmartEntity> {
        let mut results: Vec<SmartEntity> = Vec::new();
        let mut seen_values = HashSet::new();

        // Prefixed _original to silence warning
        let mut add_result = |id: &str, label: &str, icon: &str, sanitized: String, _original: &str, conf: f64, uri: Option<String>| {
            if conf >= 0.5 && !seen_values.contains(&sanitized) {
                seen_values.insert(sanitized.clone());
                let mut match_label = sanitized.clone();
                if match_label.len() > 25 {
                    match_label = format!("{}...", &match_label[..25]);
                }
                results.push(SmartEntity {
                    id: id.to_string(),
                    label: label.to_string(),
                    icon: icon.to_string(),
                    value: sanitized,
                    match_label,
                    confidence: conf,
                    uri,
                });
            }
        };

        // 1. clean_text (Applies to the whole block of text)
        let clean_orig = text;
        let clean_sanitized = sanitize_clean_text(clean_orig);
        let clean_conf = confidence_clean_text(&clean_sanitized, clean_orig);
        add_result("clean_text", "Copy Clean Text", "edit-clear-all-symbolic", clean_sanitized, clean_orig, clean_conf, None);

        // 2. url
        for m in self.url_re.find_iter(text).flatten() {
            let orig = m.as_str();
            let sanitized = orig.trim().to_string();
            add_result("url", "Copy Link", "emblem-web-symbolic", sanitized.clone(), orig, 1.0, Some(sanitized));
        }

        // 3. email
        for m in self.email_re.find_iter(text).flatten() {
            let orig = m.as_str();
            let sanitized: String = orig.chars().filter(|c| c.is_ascii_alphanumeric() || "._%+-@".contains(*c)).collect();
            let conf = if sanitized.contains('@') && sanitized.contains('.') { 1.0 } else { 0.0 };
            add_result("email", "Copy Email", "mail-unread-symbolic", sanitized.clone(), orig, conf, Some(format!("mailto:{}", sanitized)));
        }

        // 4. ipv4
        for m in self.ipv4_re.find_iter(text).flatten() {
            let orig = m.as_str();
            let sanitized: String = orig.chars().filter(|c| c.is_ascii_digit() || *c == '.' || *c == '/').collect();
            let conf = confidence_ipv4(&sanitized);
            let uri = Some(format!("http://{}", sanitized.split('/').next().unwrap_or("")));
            add_result("ipv4", "Copy IPv4", "network-wired-symbolic", sanitized, orig, conf, uri);
        }

        // 5. ipv6
        for m in self.ipv6_re.find_iter(text).flatten() {
            let orig = m.as_str();
            let s: String = orig.chars().filter(|c| !c.is_whitespace()).collect();
            let s = sanitize_hex_str(&s);
            let sanitized: String = s.chars().filter(|c| c.is_ascii_hexdigit() || ":/%_".contains(*c)).collect::<String>().to_lowercase();
            let conf = confidence_ipv6(&sanitized);
            let ip = sanitized.split('/').next().unwrap_or("").split('%').next().unwrap_or("");
            let uri = Some(format!("http://[{}]", ip));
            add_result("ipv6", "Copy IPv6", "network-server-symbolic", sanitized, orig, conf, uri);
        }

        // 6. mac
        for m in self.mac_re.find_iter(text).flatten() {
            let orig = m.as_str();
            let s: String = orig.chars().filter(|c| !c.is_whitespace()).collect();
            let s = sanitize_hex_str(&s);
            let sanitized: String = s.chars().filter(|c| c.is_ascii_hexdigit() || ":-".contains(*c)).collect::<String>().to_uppercase();
            add_result("mac", "Copy MAC Address", "network-workgroup-symbolic", sanitized, orig, 1.0, None);
        }

        // 7. iban
        for m in self.iban_re.find_iter(text).flatten() {
            let orig = m.as_str();
            let sanitized: String = orig.chars().filter(|c| c.is_ascii_alphanumeric()).collect::<String>().to_uppercase();
            let conf = if sanitized.len() >= 15 { 1.0 } else { 0.0 };
            add_result("iban", "Copy Number", "accessories-calculator-symbolic", sanitized, orig, conf, None);
        }

        // 8. uuid
        for m in self.uuid_re.find_iter(text).flatten() {
            let orig = m.as_str();
            let sanitized: String = orig.chars().filter(|c| c.is_ascii_hexdigit() || *c == '-').collect::<String>().to_lowercase();
            add_result("uuid", "Copy UUID", "fingerprint-symbolic", sanitized, orig, 1.0, None);
        }

        // 9. hex_color
        for m in self.hex_color_re.find_iter(text).flatten() {
            let orig = m.as_str();
            let s = sanitize_hex_str(orig);
            let sanitized: String = s.chars().filter(|c| c.is_ascii_hexdigit() || *c == '#').collect::<String>().to_uppercase();
            add_result("hex_color", "Copy Hex Color", "color-select-symbolic", sanitized, orig, 1.0, None);
        }

        // 10. date
        for m in self.date_re.find_iter(text).flatten() {
            let orig = m.as_str();
            let sanitized: String = orig.chars().filter(|c| c.is_ascii_digit() || "-/".contains(*c)).collect();
            add_result("date", "Copy Date", "x-office-calendar-symbolic", sanitized, orig, 1.0, None);
        }

        // 11. phone
        for m in self.phone_re.find_iter(text).flatten() {
            let orig = m.as_str();
            let sanitized: String = orig.chars().filter(|c| c.is_ascii_digit() || *c == '+').collect();
            let conf = confidence_phone(&sanitized, orig);
            let cleaned: String = sanitized.chars().filter(|c| c.is_ascii_digit() || *c == '+').collect();
            let uri = Some(format!("tel:{}", cleaned));
            add_result("phone", "Copy Phone", "call-start-symbolic", sanitized, orig, conf, uri);
        }

        // 12. number
        for m in self.number_re.find_iter(text).flatten() {
            let orig = m.as_str();
            let sanitized: String = orig.chars().filter(|c| c.is_ascii_digit()).collect();
            let conf = confidence_number(&sanitized, orig);
            add_result("number", "Copy Number", "accessories-calculator-symbolic", sanitized, orig, conf, None);
        }

        results.sort_by(|a, b| b.confidence.partial_cmp(&a.confidence).unwrap_or(std::cmp::Ordering::Equal));
        results
    }
}

// ---------------------------------------------------------
// Helper functions porting JavaScript logic directly
// ---------------------------------------------------------

fn sanitize_hex_str(s: &str) -> String {
    s.chars().map(|c| match c {
        'S' | 's' => '5',
        'O' | 'o' => '0',
        'I' | 'i' | 'L' | 'l' => '1',
        'Z' | 'z' => '2',
        'G' | 'g' => '6',
        'T' | 't' => '7',
        'Q' | 'q' => '0',
        _ => c,
    }).collect()
}

fn sanitize_clean_text(s: &str) -> String {
    let mut cleaned_lines = Vec::new();
    let force_re = Regex::new(r"^[-/]{1,2}[a-zA-Z0-9]+$").unwrap();

    for line in s.lines() {
        let tokens: Vec<&str> = line.split_whitespace().collect();
        if tokens.is_empty() { continue; }

        let mut token_qualities = Vec::new();
        for t in &tokens {
            if t.len() == 1 && "-:|".contains(t.chars().next().unwrap()) {
                token_qualities.push(1);
                continue;
            }

            let alpha_num = t.chars().filter(|c| c.is_ascii_alphanumeric()).count();
            if alpha_num == 0 {
                token_qualities.push(0);
                continue;
            }

            if force_re.is_match(t).unwrap_or(false) {
                token_qualities.push(2);
                continue;
            }

            if t.len() == 1 {
                if "aAI0123456789".contains(t.chars().next().unwrap()) {
                    token_qualities.push(2);
                } else {
                    token_qualities.push(0);
                }
                continue;
            }

            let letters: String = t.chars().filter(|c| c.is_ascii_alphabetic()).collect();
            if letters.len() == 2 {
                let mut chars = letters.chars();
                let first = chars.next().unwrap();
                let second = chars.next().unwrap();
                if first.to_ascii_lowercase() == second.to_ascii_lowercase() && first != second {
                    token_qualities.push(0);
                    continue;
                }
            }

            if (alpha_num as f64) / (t.len() as f64) < 0.5 {
                token_qualities.push(0);
                continue;
            }

            let has_vowel_or_digit = t.chars().any(|c| "aeiouyAEIOUY0123456789".contains(c));
            let is_acronym = !letters.is_empty() && letters.chars().all(|c| c.is_ascii_uppercase());

            if !has_vowel_or_digit && !is_acronym && letters.len() <= 2 {
                token_qualities.push(0);
                continue;
            }

            token_qualities.push(2);
        }

        let mut best_start = -1_isize;
        let mut best_end = -1_isize;
        let mut max_score = -1_isize;

        for i in 0..tokens.len() {
            if token_qualities[i] == 2 {
                for j in i..tokens.len() {
                    if token_qualities[j] == 2 {
                        let mut valid = true;
                        let mut score = 0;
                        for k in i..=j {
                            if token_qualities[k] == 0 {
                                valid = false;
                                break;
                            }
                            if token_qualities[k] == 2 {
                                score += tokens[k].len() as isize;
                            }
                        }
                        if valid && score > max_score {
                            max_score = score;
                            best_start = i as isize;
                            best_end = j as isize;
                        }
                    }
                }
            }
        }

        if best_start != -1 && best_end != -1 {
            cleaned_lines.push(tokens[(best_start as usize)..=(best_end as usize)].join(" "));
        }
    }

    cleaned_lines.join("\n")
}

fn confidence_clean_text(sanitized: &str, original: &str) -> f64 {
    if sanitized.len() < 4 { return 0.0; }
    if sanitized.lines().count() > 3 { return 0.0; }
    let orig_trimmed = original.trim();
    let removed_chars = (orig_trimmed.len() as isize) - (sanitized.len() as isize);
    if removed_chars >= 3 && sanitized != orig_trimmed && !sanitized.is_empty() {
        return 0.85;
    }
    0.0
}

fn confidence_ipv4(sanitized: &str) -> f64 {
    let parts: Vec<&str> = sanitized.split('/').collect();
    let ip = parts[0];
    let ip_parts: Vec<&str> = ip.split('.').collect();
    if ip_parts.len() != 4 { return 0.0; }
    let valid = ip_parts.iter().all(|p| {
        if p.is_empty() { return false; }
        p.parse::<u8>().is_ok()
    });
    if !valid { return 0.0; }
    if parts.len() > 1 {
        if let Ok(sub) = parts[1].parse::<u8>() {
            if sub > 32 { return 0.0; }
        } else { return 0.0; }
    }
    let ignored = ["0.0.0.0", "1.0.0.0", "1.2.3.4"];
    if ignored.contains(&ip) { return 0.4; }
    1.0
}

fn confidence_ipv6(sanitized: &str) -> f64 {
    if sanitized == "::" || sanitized == "::/0" { return 0.0; }
    let ip_part = sanitized.split('/').next().unwrap_or("");
    let ip = ip_part.split('%').next().unwrap_or("");
    let parts: Vec<&str> = ip.split(':').collect();
    if parts.len() <= 2 { return 0.0; }
    if let Some(subnet_str) = sanitized.split('/').nth(1) {
        if let Ok(sub) = subnet_str.parse::<u8>() {
            if sub > 128 { return 0.0; }
        } else { return 0.0; }
    }
    1.0
}

fn confidence_phone(sanitized: &str, original: &str) -> f64 {
    let dot_count = original.chars().filter(|&c| c == '.').count();
    let colon_count = original.chars().filter(|&c| c == ':').count();
    let slash_count = original.chars().filter(|&c| c == '/').count();
    if dot_count >= 2 || colon_count >= 2 || slash_count >= 1 { return 0.0; }

    let cleaned: String = sanitized.chars().filter(|c| c.is_ascii_digit() || *c == '+').collect();
    if cleaned.len() < 7 || cleaned.len() > 15 { return 0.0; }

    let has_formatting = original.contains(' ') || original.contains('-');
    if cleaned.starts_with('+') || cleaned.starts_with("00") { return 0.9; }
    if has_formatting { return 0.7; }
    if cleaned.starts_with('0') && cleaned.len() >= 10 && cleaned.len() <= 11 { return 0.8; }
    0.4
}

fn confidence_number(sanitized: &str, original: &str) -> f64 {
    let cleaned: String = sanitized.chars().filter(|c| c.is_ascii_digit()).collect();
    if cleaned.len() < 5 { return 0.0; }
    let mult_delimiters = original.chars().filter(|&c| "-./".contains(c)).count();
    if mult_delimiters >= 2 { return 0.2; }
    if cleaned.len() >= 8 { return 0.6; }
    0.4
}