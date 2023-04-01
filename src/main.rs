use std::{collections::HashMap, env, sync::Arc};

use chrono::Local;
use futures::{stream::StreamExt, SinkExt};
use reqwest::{Client, Url};
use serde_json::{json, Value};
use tokio::sync::Mutex;
use tokio_tungstenite::{connect_async, tungstenite::Message};
use tracing_appender::{non_blocking, rolling};
use tracing_subscriber::{
    fmt::{self, time::FormatTime, writer::MakeWriterExt},
    prelude::__tracing_subscriber_SubscriberExt,
    util::SubscriberInitExt,
    EnvFilter, Registry,
};

type Error = Box<dyn std::error::Error + Send + Sync>;

type Session = Arc<Mutex<HashMap<String, Vec<Value>>>>;

struct LocalTimer;
impl FormatTime for LocalTimer {
    fn format_time(&self, w: &mut fmt::format::Writer<'_>) -> std::fmt::Result {
        write!(w, "{}", Local::now().format("%FT%T%.3f"))
    }
}

#[tokio::main]
pub async fn main() -> Result<(), Error> {
    let (debug_file, _guard) = non_blocking(rolling::daily("logs", "debug"));
    let (warn_file, _guard) = non_blocking(rolling::daily("logs", "warning"));
    let (info_file, _guard) = non_blocking(rolling::daily("logs", "info"));
    let all_files = debug_file
        .and(
            warn_file
                .with_max_level(tracing::Level::WARN)
                .with_min_level(tracing::Level::ERROR),
        )
        .and(
            info_file
                .with_max_level(tracing::Level::INFO)
                .with_min_level(tracing::Level::INFO),
        );
    Registry::default()
        .with(EnvFilter::from_default_env().add_directive(tracing::Level::INFO.into()))
        .with(
            fmt::layer()
                .pretty()
                .with_writer(std::io::stdout)
                .with_timer(LocalTimer),
        )
        .with(
            fmt::layer()
                .with_timer(LocalTimer)
                .with_ansi(false)
                .with_writer(all_files),
        )
        .init();
    let mattermost_token = env::var("MATTERMOST_TOKEN").expect("ENV MATTERMOST_TOKEN");
    let openai_api_key = env::var("OPENAI_API_KEY").expect("ENV OPENAI_API_KEY");
    let most_url = Url::parse(&env::var("MATTERMOST_URL").expect("ENV MATTERMOST_URL"))?;
    let bot_name = env::var("MATTERMOST_BOT_NAME").unwrap_or("chatgpt".to_owned());
    let ws_url = match most_url.scheme() {
        "https" => format!("wss://{}", most_url.host().expect("MATTERMOST_URL INVALID")),
        "http" => format!("ws://{}", most_url.host().expect("MATTERMOST_URL INVALID")),
        _ => return Err("MATTERMOST_URL INVALID".into()),
    };

    let (mut socket, _) = connect_async(format!("{}/api/v4/websocket", ws_url))
        .await
        .expect("Failed to connect");

    // Authenticate with the Mattermost server
    let auth_message = json!({
        "seq": 1,
        "action": "authentication_challenge",
        "data": {
            "token": mattermost_token,
        },
    });
    socket.send(Message::Text(auth_message.to_string())).await?;
    let client = match env::var("OPENAI_API_PROXY") {
        Ok(proxy) => reqwest::Client::builder()
            .proxy(reqwest::Proxy::https(proxy)?)
            .build()?,
        Err(_) => reqwest::Client::new(),
    };
    let mut most = Most::new(&mattermost_token, &most_url.to_string(), &bot_name);
    most.me().await?;
    tracing::info!("开始处理...");
    let session = Arc::new(Mutex::new(HashMap::new()));
    loop {
        let message = socket.next().await.expect("Failed to receive message")?;
        let most_gpt = MostGPT::new(most.clone(), &openai_api_key, client.clone());
        let session = session.clone();
        tokio::spawn(async move {
            match message {
                Message::Text(text) => match most_gpt.process_text(&text, session).await {
                    Ok(_) => {}
                    Err(e) => tracing::error!("{:?}", e),
                },
                _ => {}
            }
        });
    }
}

#[derive(Clone)]
struct Most {
    client: Client,
    bot_token: String,
    base_url: String,
    user_id: String,
    bot_name: String,
}

impl Most {
    pub fn new(bot_token: &str, base_url: &str, bot_name: &str) -> Self {
        Most {
            client: reqwest::Client::new(),
            bot_token: bot_token.to_owned(),
            base_url: base_url.to_owned(),
            user_id: "".to_string(),
            bot_name: bot_name.to_string(),
        }
    }
    pub async fn me(&mut self) -> Result<(), Error> {
        let res = self
            .client
            .get(format!("{}api/v4/users/me", self.base_url))
            .bearer_auth(&self.bot_token)
            .send()
            .await?;
        let j = res.json::<Value>().await?;
        self.user_id = j["id"].as_str().ok_or("No id found")?.to_owned();
        tracing::debug!("{:?}", j);
        Ok(())
    }
    pub async fn posts(
        &self,
        channel_id: &str,
        message: &Value,
        root_id: &str,
    ) -> Result<(), Error> {
        self.client
            .post(format!("{}api/v4/posts", self.base_url))
            .bearer_auth(&self.bot_token)
            .header("Content-Type", "application/json")
            .json(&json!({
                "channel_id": channel_id,
                "message": message,
                "root_id": root_id
            }))
            .send()
            .await?;
        Ok(())
    }
}

struct MostGPT {
    most: Most,
    gpt_api_key: String,
    proxy_client: Client,
}

impl MostGPT {
    fn new(most: Most, gpt_api_key: &str, proxy_client: Client) -> Self {
        Self {
            most: most,
            gpt_api_key: gpt_api_key.to_owned(),
            proxy_client,
        }
    }
    async fn process_text(&self, text: &str, session: Session) -> Result<(), Error> {
        let message = serde_json::from_str::<Value>(text)?;

        if message["seq"].as_i64() == Some(1) && message["event"].as_str() == Some("hello") {
            tracing::info!("Connected to Mattermost");
        } else if message["event"].as_str() == Some("posted") {
            // tracing::info!("received: {:?}", message);
            let post: Value = serde_json::from_str(message["data"]["post"].as_str().unwrap())?;
            let user_id = post["user_id"].as_str().ok_or("No user ID found")?;
            let channel_id = post["channel_id"].as_str().ok_or("No channel ID found")?;
            let sender_name = message["data"]["sender_name"].as_str().unwrap_or("none");
            let channel_name = message["data"]["channel_name"]
                .as_str()
                .ok_or("No channel_name found")?;
            if user_id != self.most.user_id {
                let message_text = post["message"].as_str().ok_or("No message text found")?;
                let post_id = post["id"].as_str().ok_or("No post id found")?;
                if message_text.contains(&format!("@{}", self.most.bot_name))
                    || channel_name.contains(&self.most.user_id)
                {
                    if message_text.contains("clear ctx") {
                        session.lock().await.clear();
                        self.most
                            .posts(
                                channel_id,
                                &Value::String("已清空上下文信息".to_owned()),
                                post_id,
                            )
                            .await?;
                        return Ok(());
                    }
                    self.most
                        .posts(
                            channel_id,
                            &Value::String("正在处理中...".to_owned()),
                            post_id,
                        )
                        .await?;
                    session
                        .lock()
                        .await
                        .entry(user_id.to_owned())
                        .or_insert(vec![]);
                    let mut send = vec![json!({
                        "role": "system",
                        "content": format!("您是一个名为{}的有用助手，以Markdown格式提供简洁的答案", self.most.bot_name)
                    })];
                    session
                        .lock()
                        .await
                        .get(user_id)
                        .unwrap()
                        .iter()
                        .rev()
                        .take(10) // 保留 5 次对话的上下文
                        .rev()
                        .for_each(|h| send.push(h.clone()));
                    let this_send = json!({
                        "role": "user",
                        "content": message_text
                    });
                    send.push(this_send.clone());
                    tracing::info!("sender_name: {}, question: {}", sender_name, message_text);
                    match self
                        .proxy_client
                        // .post("https://api.openai.com/v1/chat/completions")
                        .post("https://www.sqlchat.ai/api/chat")
                        // .bearer_auth(&self.gpt_api_key)
                        // .header("Content-Type", "application/json")
                        .json(&json!({
                            "messages": send,
                            // "model": "gpt-3.5-turbo",
                            // "max_tokens": 1000
                        }))
                        .send()
                        .await
                    {
                        Ok(gpt_res) => {
                            // let v = gpt_res.json::<Value>().await?;
                            // let answer = &v["choices"][0]["message"]["content"];
                            let answer = &Value::String(gpt_res.text().await?);
                            match answer {
                                Value::Null => {
                                    tracing::warn!("sender_name: {}, 上下文过长", sender_name);
                                    session.lock().await.get_mut(user_id).unwrap().reverse();
                                    session.lock().await.get_mut(user_id).unwrap().truncate(4);
                                    session.lock().await.get_mut(user_id).unwrap().reverse();
                                    self.most
                                        .posts(
                                            channel_id,
                                            &Value::String(
                                                "上下文过长，已截断两次对话前的上下文，请重新提问，你也可以发送 'clear ctx' 来清空上下文"
                                                    .to_owned(),
                                            ),
                                            post_id,
                                        )
                                        .await?;
                                }
                                _ => {
                                    tracing::info!(
                                        "sender_name: {}, answer: {}",
                                        sender_name,
                                        answer
                                    );
                                    // tracing::info!(
                                    //     "sender_name: {}, usage: {}",
                                    //     sender_name,
                                    //     &v["usage"]
                                    // );
                                    session.lock().await.get_mut(user_id).unwrap().extend(vec![
                                        this_send,
                                        json!({"role": "assistant", "content": answer}),
                                    ]);
                                    self.most.posts(channel_id, answer, post_id).await?;
                                }
                            }
                        }
                        Err(e) => {
                            self.most
                                .posts(channel_id, &Value::String(e.to_string()), post_id)
                                .await?;
                        }
                    }
                }
            }
        }
        Ok(())
    }
}
