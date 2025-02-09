use axum::{routing::get, Router};
use ngrok::prelude::*;
use slack_rs::{
    create_app_with_path, Event, MessageClient, SigningSecret, SlackEventHandler, Token,
};
use std::net::SocketAddr;
use tracing::{info, Level};
use tracing_subscriber::FmtSubscriber;

// メンション応答ハンドラの定義
#[derive(Clone)]
struct MentionHandler;

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
                client
                    .reply_to_thread(&channel, &ts, "はい、呼びましたか？")
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

    // ルーターの設定
    let router = Router::new()
        .route("/health", get(|| async { "OK" }))
        .merge(create_app_with_path(
            SigningSecret::new(signing_secret),
            bot_token,
            MentionHandler,
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
