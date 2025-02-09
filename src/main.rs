use axum::{routing::get, Router};
use ngrok::prelude::*;
use slack_rs::{
    create_app_with_path, Event, MessageClient, SigningSecret, SlackEventHandler, Token,
};
use std::net::SocketAddr;
use tracing::{info, Level};
use tracing_subscriber::FmtSubscriber;
use serde_json::{json, Value};
use reqwest;

// LLMクライアント構造体
#[derive(Clone)]
struct LLMClient {
    api_url: String,
    operator_id: String,
    api_token: String,
}

impl LLMClient {
    fn new(api_url: String, operator_id: String, api_token: String) -> Self {
        Self {
            api_url,
            operator_id,
            api_token,
        }
    }

    async fn get_response(&self, user_message: &str) -> Result<String, Box<dyn std::error::Error>> {
        let client = reqwest::Client::new();
        
        let request_body = json!({
            "model": "google_ai:gemini-2.0-flash-exp",
            "messages": [
                {
                    "role": "developer",
                    "content": "You are a helpful assistant that responds in Japanese."
                },
                {
                    "role": "user",
                    "content": user_message
                }
            ]
        });

        let response = client
            .post(&self.api_url)
            .header("Accept", "application/json")
            .header("x-operator-id", &self.operator_id)
            .header("Content-Type", "application/json")
            .header("Authorization", format!("Bearer {}", self.api_token))
            .json(&request_body)
            .send()
            .await?;

        let response_json: Value = response.json().await?;
        let content = response_json["choices"][0]["message"]["content"]
            .as_str()
            .unwrap_or("申し訳ありません。応答を生成できませんでした。")
            .to_string();

        Ok(content)
    }
}

// メンションハンドラの定義
#[derive(Clone)]
struct MentionHandler {
    llm_client: LLMClient,
}

impl MentionHandler {
    fn new(llm_client: LLMClient) -> Self {
        Self { llm_client }
    }
}

#[async_trait::async_trait]
impl SlackEventHandler for MentionHandler {
    async fn handle_event(
        &self,
        event: Event,
        client: &MessageClient,
    ) -> Result<(), Box<dyn std::error::Error>> {
        match event {
            Event::AppMention {
                channel, ts, text, ..
            } => {
                info!(
                    "メンションを受信: channel={}, ts={}, text={}",
                    channel, ts, text
                );

                // LLM APIから応答を取得
                let response = self.llm_client.get_response(&text).await?;

                // Slackに返信
                client
                    .reply_to_thread(&channel, &ts, &response)
                    .await?;
            }
            _ => info!("未対応のイベント: {:?}", event),
        }
        Ok(())
    }
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    dotenvy::dotenv().ok();

    // ロギングの初期化
    FmtSubscriber::builder()
        .with_max_level(Level::INFO)
        .compact()
        .init();

    info!("メンション応答サーバーを起動します");

    // 環境変数からSlack認証情報を取得
    let signing_secret =
        std::env::var("SLACK_SIGNING_SECRET").expect("SLACK_SIGNING_SECRETが設定されていません");
    let bot_token = std::env::var("SLACK_BOT_TOKEN").expect("SLACK_BOT_TOKENが設定されていません");
    let bot_token = Token::new(bot_token);

    let ngrok_domain = std::env::var("NGROK_DOMAIN").expect("NGROK_DOMAINが設定されていません");

    // LLM APIの設定を環境変数から取得
    let api_url = std::env::var("API_URL").expect("API_URLが設定されていません");
    let operator_id = std::env::var("OPERATOR_ID").expect("OPERATOR_IDが設定されていません");
    let api_token = std::env::var("API_TOKEN").expect("API_TOKENが設定されていません");

    let llm_client = LLMClient::new(api_url, operator_id, api_token);

    // ルーターの設定
    let router = Router::new()
        .route("/health", get(|| async { "OK" }))
        .merge(create_app_with_path(
            SigningSecret::new(signing_secret),
            bot_token,
            MentionHandler::new(llm_client),
            "/push",
        ));

    // サーバーアドレスの設定
    let addr = SocketAddr::from(([127, 0, 0, 1], 3000));
    info!("サーバーを開始します: {}", addr);

    let tun = ngrok::Session::builder()
        // NGROKトークンを環境変数から読み込み
        .authtoken_from_env()
        // NGROKセッションの接続
        .connect()
        .await?
        // HTTPエンドポイントのトンネルを開始
        .http_endpoint()
        .domain(ngrok_domain)
        .listen()
        .await?;

    info!("Tunnel URL: {}", tun.url());

    // サーバーの起動
    axum::Server::builder(tun)
        .serve(router.into_make_service_with_connect_info::<SocketAddr>())
        .await
        .unwrap();

    Ok(())
}
