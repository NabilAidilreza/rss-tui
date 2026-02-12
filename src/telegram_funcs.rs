use grammers_client::SignInError;
use grammers_client::Client;
use grammers_session::storages::SqliteSession;
use grammers_mtsender::SenderPool;
use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use tokio::sync::mpsc;
use std::io::{self, Write};

pub struct TelegramMonitor {
    pub last_seen: Arc<Mutex<HashMap<i64, i32>>>,
}

impl TelegramMonitor {
    pub fn new() -> Self {
        Self {
            last_seen: Arc::new(Mutex::new(HashMap::new())),
        }
    }

    pub async fn create_client(&self, api_id: i32) -> anyhow::Result<Client> {
        let session = Arc::new(SqliteSession::open("telegram.session")?);
        let pool = SenderPool::new(Arc::clone(&session), api_id);
        let client = Client::new(&pool);

        // 1. You MUST take the runner out of the pool
        let runner = pool.runner;

        // 2. You MUST move the runner into the spawned task
        tokio::spawn(async move {
            // We handle the result here so the block returns ()
            let _ = runner.run().await;
        }); 

        Ok(client)
    }

    pub async fn ensure_authorized(&self, client: &Client, api_hash: &str) -> anyhow::Result<()> {
        if !client.is_authorized().await? {
            println!("--- Telegram Login Required ---");
            print!("Enter phone (e.g. +123456789): ");
            io::stdout().flush()?;
            let mut phone = String::new();
            io::stdin().read_line(&mut phone)?;
            let phone = phone.trim();

            // 1. Request the login code
            let token = client.request_login_code(phone, api_hash).await?;
            
            print!("Enter the code sent to your Telegram: ");
            io::stdout().flush()?;
            let mut code = String::new();
            io::stdin().read_line(&mut code)?;
            let code = code.trim();

            // 2. Attempt sign in
            let login_result = client.sign_in(&token, code).await;
            
            match login_result {
                Ok(user) => {
                    println!("Signed in as {}!", user.first_name().unwrap_or("User"));
                }
                Err(SignInError::PasswordRequired(password_token)) => {
                    // 3. Handle 2FA (Two-Factor Authentication)
                    println!("2FA Required. Hint: {}", password_token.hint().unwrap_or("None"));
                    print!("Enter 2FA password: ");
                    io::stdout().flush()?;
                    let mut password = String::new();
                    io::stdin().read_line(&mut password)?;
                    
                    client.check_password(password_token, password.trim()).await?;
                    println!("✅ 2FA Login successful!");
                }
                Err(e) => return Err(anyhow::anyhow!("Login failed: {}", e)),
            };
            
        }
        Ok(())
    }

    pub async fn monitor(
        &self, 
        client: Client, 
        target_chat_ids: Vec<i64>, 
        ui_tx: mpsc::UnboundedSender<(String, String)> 
    ) -> anyhow::Result<()> {
        loop {
            // We re-fetch the dialog list each iteration to catch new messages
            let mut dialogs = client.iter_dialogs();

            while let Some(dialog) = dialogs.next().await? {
                let peer = dialog.peer();
                let chat_id = peer.id().bot_api_dialog_id();

                if !target_chat_ids.contains(&chat_id) {
                    continue;
                }

                if let Some(msg) = dialog.last_message.as_ref() {
                    let msg_id = msg.id();

                    // Deduplication logic using the Mutex-wrapped last_seen map
                    {
                        let mut last_seen = self.last_seen.lock().unwrap();
                        if let Some(&prev_id) = last_seen.get(&chat_id) {
                            if msg_id <= prev_id { continue; }
                        }
                        last_seen.insert(chat_id, msg_id);
                    }

                    let sender_name = peer.name()
                        .map(|s| s.to_owned())
                        .unwrap_or_else(|| "Unknown".to_string());

                    let clean_text = msg.text().replace('\n', " ");
                    
                    // Send to the channel which main.rs is listening to
                    let _ = ui_tx.send((sender_name, clean_text));
                }
            }

            // Wait for 2 seconds before checking for new "Latest Messages" again
            tokio::time::sleep(std::time::Duration::from_secs(2)).await;
        }
    }
}