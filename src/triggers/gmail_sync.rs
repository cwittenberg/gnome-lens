// src/triggers/gmail_sync.rs
use std::fs;
use std::path::{Path, PathBuf};
use std::thread;
use std::time::{Duration, SystemTime, UNIX_EPOCH};
use native_tls::TlsConnector;

#[cfg(unix)]
use std::os::unix::fs::PermissionsExt;

pub struct GmailSyncDaemon {
    config_path: PathBuf,
    mail_dir: PathBuf,
    state_file: PathBuf,
}

impl GmailSyncDaemon {
    pub fn new(config_dir: &str, data_dir: &str) -> Self {
        let mail_dir = Path::new(data_dir).join("mail");
        if !mail_dir.exists() {
            let _ = fs::create_dir_all(&mail_dir);
            #[cfg(unix)]
            {
                if let Ok(mut perms) = fs::metadata(&mail_dir).map(|m| m.permissions()) {
                    perms.set_mode(0o700);
                    let _ = fs::set_permissions(&mail_dir, perms);
                }
            }
        }

        Self {
            config_path: Path::new(config_dir).join("gmail.json"),
            mail_dir,
            state_file: Path::new(config_dir).join("gmail_state.json"),
        }
    }

    pub fn start(&self) {
        let config_path = self.config_path.clone();
        let mail_dir = self.mail_dir.clone();
        let state_file = self.state_file.clone();

        thread::spawn(move || {
            loop {
                if let Ok(config_data) = fs::read_to_string(&config_path) {
                    if let Ok(config) = serde_json::from_str::<serde_json::Value>(&config_data) {
                        if let (Some(email), Some(password)) = (config["email"].as_str(), config["app_password"].as_str()) {
                            let history_years = config["history_years"].as_u64().unwrap_or(1).clamp(1, 5) as u32;
                            Self::sync_inbox(email, password, &mail_dir, &state_file, history_years);
                        } else {
                            Self::write_state(&state_file, 0, false, 0, 0, "Missing credentials.", true, vec![]);
                        }
                    }
                } else {
                    let dummy_config = serde_json::json!({
                        "email": "your_email@gmail.com",
                        "app_password": "your_16_char_app_password_here",
                        "history_years": 1
                    });
                    if let Ok(json_str) = serde_json::to_string_pretty(&dummy_config) {
                        let _ = fs::write(&config_path, json_str);
                        #[cfg(unix)]
                        {
                            if let Ok(mut perms) = fs::metadata(&config_path).map(|m| m.permissions()) {
                                perms.set_mode(0o600);
                                let _ = fs::set_permissions(&config_path, perms);
                            }
                        }
                    }
                    Self::write_state(&state_file, 0, false, 0, 0, "Awaiting user configuration.", false, vec![]);
                }
                thread::sleep(Duration::from_secs(60));
            }
        });
    }

    fn write_state(state_file: &Path, last_uid: u32, is_syncing: bool, total: usize, current: usize, message: &str, is_error: bool, uncommitted: Vec<u32>) {
        let state = serde_json::json!({
            "last_uid": last_uid,
            "is_syncing": is_syncing,
            "total_emails": total,
            "synced_emails": current,
            "message": message,
            "is_error": is_error,
            "uncommitted_backlog": uncommitted
        });
        let _ = fs::write(state_file, serde_json::to_string(&state).unwrap());
    }

    fn sync_inbox(email: &str, password: &str, mail_dir: &Path, state_file: &Path, history_years: u32) {
        let mut last_uid = 0;
        let mut uncommitted_backlog = Vec::new();
        
        if let Ok(state_data) = fs::read_to_string(state_file) {
            if let Ok(state_json) = serde_json::from_str::<serde_json::Value>(&state_data) {
                last_uid = state_json["last_uid"].as_u64().unwrap_or(0) as u32;
                if let Some(arr) = state_json["uncommitted_backlog"].as_array() {
                    uncommitted_backlog = arr.iter().filter_map(|v| v.as_u64().map(|n| n as u32)).collect();
                }
            }
        }

        Self::write_state(state_file, last_uid, true, 0, 0, "Connecting to IMAP securely...", false, uncommitted_backlog.clone());

        let domain = "imap.gmail.com";
        let tls_res = TlsConnector::builder().build();
        if tls_res.is_err() {
            Self::write_state(state_file, last_uid, false, 0, 0, "TLS connection generation failed", true, uncommitted_backlog);
            return;
        }
        let tls = tls_res.unwrap();
        
        let client = match imap::connect((domain, 993), domain, &tls) {
            Ok(c) => c,
            Err(e) => {
                let err_msg = format!("Network Connection Error: {}", e);
                Self::write_state(state_file, last_uid, false, 0, 0, &err_msg, true, uncommitted_backlog);
                return;
            }
        };

        let mut session = match client.login(email, password) {
            Ok(s) => s,
            Err((_, _)) => {
                let err_msg = format!("Login Rejected: Invalid Email or App Password.");
                Self::write_state(state_file, last_uid, false, 0, 0, &err_msg, true, uncommitted_backlog);
                return;
            }
        };

        if let Err(e) = session.select("INBOX") {
            let err_msg = format!("Folder Selection Error: {}", e);
            Self::write_state(state_file, last_uid, false, 0, 0, &err_msg, true, uncommitted_backlog);
            return;
        }

        let mut uids = if !uncommitted_backlog.is_empty() {
            uncommitted_backlog.clone()
        } else {
            let query = if last_uid == 0 {
                let secs = SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_secs();
                let current_year = 1970 + (secs / 31556926); 
                let target_year = current_year - history_years as u64;
                format!("SINCE 01-Jan-{}", target_year)
            } else {
                format!("UID {}:*", last_uid + 1)
            };

            let mut fetched_uids: Vec<u32> = session.uid_search(&query).unwrap_or_default().into_iter().collect();
            fetched_uids.sort_unstable();
            fetched_uids.retain(|&u| u > last_uid);
            fetched_uids
        };

        let total_unindexed_count = uids.len();
        if total_unindexed_count == 0 {
            Self::write_state(state_file, last_uid, false, 0, 0, "Inbox is fully synchronized.", false, vec![]);
            let _ = session.logout();
            return;
        }

        let processing_backlog = total_unindexed_count > 500;
        if processing_backlog && uncommitted_backlog.is_empty() {
            uids = uids.into_iter().take(500).collect();
        }

        let mut active_uncommitted = uids.clone();
        Self::write_state(state_file, last_uid, true, total_unindexed_count, 0, "Synchronizing messages...", false, active_uncommitted.clone());

        let mut downloaded_count = 0;
        let mut tracking_highest_uid = last_uid;

        for chunk in uids.chunks(50) {
            let fetch_query = chunk.iter().map(|u| u.to_string()).collect::<Vec<_>>().join(",");
            
            match session.uid_fetch(&fetch_query, "RFC822") {
                Ok(messages) => {
                    for msg in messages.iter() {
                        let uid = msg.uid.unwrap_or(0);
                        if let Some(body) = msg.body() {
                            let file_path = mail_dir.join(format!("{}.eml", uid));

                            if fs::write(&file_path, body).is_ok() {
                                #[cfg(unix)]
                                {
                                    if let Ok(mut perms) = fs::metadata(&file_path).map(|m| m.permissions()) {
                                        perms.set_mode(0o600);
                                        let _ = fs::set_permissions(&file_path, perms);
                                    }
                                }
                                downloaded_count += 1;
                                active_uncommitted.retain(|&x| x != uid);
                            }
                        }
                        if uid > tracking_highest_uid {
                            tracking_highest_uid = uid;
                        }
                    }
                }
                Err(e) => {
                    let err_msg = format!("Mime Batch Fetch Error: {}", e);
                    Self::write_state(state_file, last_uid, false, total_unindexed_count, downloaded_count, &err_msg, true, active_uncommitted);
                    return;
                }
            }

            Self::write_state(
                state_file,
                if active_uncommitted.is_empty() && !processing_backlog { tracking_highest_uid } else { last_uid },
                true,
                total_unindexed_count,
                downloaded_count,
                &format!("Syncing: pulled {} / {} entries", downloaded_count, total_unindexed_count),
                false,
                active_uncommitted.clone()
            );
        }

        let final_committed_uid = if active_uncommitted.is_empty() { tracking_highest_uid } else { last_uid };
        
        Self::write_state(
            state_file,
            final_committed_uid,
            false,
            total_unindexed_count,
            downloaded_count,
            if processing_backlog { "Catching up backlog window..." } else { "Inbox is fully synchronized." },
            false,
            active_uncommitted
        );

        let _ = session.logout();
    }
}