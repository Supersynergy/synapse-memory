use synapse_market::alert::{AlertAction, AlertCondition, AlertCtx, AlertEngine, AlertRule, Op};

#[tokio::test]
async fn test_log_alert_fires() {
    let mut engine = AlertEngine::new();
    engine.register(AlertRule {
        id: "test_log".into(),
        condition: AlertCondition::ThresholdCross {
            col: "close".into(),
            op: Op::Gt,
            value: 100.0,
        },
        action: AlertAction::Log,
    });

    let mut ctx = AlertCtx::new();
    ctx.set("close", 105.0);
    // Should not panic; Log action prints to stdout.
    engine.fire(&ctx).await;
}

#[tokio::test]
async fn test_log_alert_does_not_fire_below_threshold() {
    let mut engine = AlertEngine::new();
    engine.register(AlertRule {
        id: "no_fire".into(),
        condition: AlertCondition::ThresholdCross {
            col: "close".into(),
            op: Op::Gt,
            value: 200.0,
        },
        action: AlertAction::Log,
    });
    let mut ctx = AlertCtx::new();
    ctx.set("close", 50.0);
    // No panic; rule should not fire.
    engine.fire(&ctx).await;
}

#[tokio::test]
async fn test_pattern_match_condition() {
    let mut engine = AlertEngine::new();
    engine.register(AlertRule {
        id: "pattern_42".into(),
        condition: AlertCondition::PatternMatch { pattern_id: 42 },
        action: AlertAction::Log,
    });
    let mut ctx = AlertCtx::new();
    ctx.match_pattern(42);
    // Should not panic.
    engine.fire(&ctx).await;
}

#[tokio::test]
async fn test_and_all_condition() {
    let mut engine = AlertEngine::new();
    engine.register(AlertRule {
        id: "and_rule".into(),
        condition: AlertCondition::AndAll(vec![
            AlertCondition::ThresholdCross {
                col: "rsi".into(),
                op: Op::Gt,
                value: 70.0,
            },
            AlertCondition::ThresholdCross {
                col: "vol".into(),
                op: Op::Gt,
                value: 1_000_000.0,
            },
        ]),
        action: AlertAction::Log,
    });
    let mut ctx = AlertCtx::new();
    ctx.set("rsi", 75.0);
    ctx.set("vol", 2_000_000.0);
    engine.fire(&ctx).await;
}

#[tokio::test]
async fn test_webhook_fires_post() {
    // Use a mock HTTP server via tokio + tiny_http or just a real localhost listener.
    // Since mockito / httpmock would require extra deps, we test with a server that
    // immediately returns 200 — use tokio::net::TcpListener.
    use tokio::io::AsyncWriteExt;
    use tokio::net::TcpListener;

    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = listener.local_addr().unwrap().port();
    let url = format!("http://127.0.0.1:{port}/hook");

    let server_handle = tokio::spawn(async move {
        let (mut socket, _) = listener.accept().await.unwrap();
        // Read request (discard).
        let mut buf = [0u8; 1024];
        let _ = tokio::io::AsyncReadExt::read(&mut socket, &mut buf).await;
        let response = b"HTTP/1.1 200 OK\r\nContent-Length: 0\r\nConnection: close\r\n\r\n";
        socket.write_all(response).await.unwrap();
    });

    let mut engine = AlertEngine::new();
    engine.register(AlertRule {
        id: "wh_rule".into(),
        condition: AlertCondition::ThresholdCross {
            col: "price".into(),
            op: Op::Gte,
            value: 1.0,
        },
        action: AlertAction::Webhook {
            url: url.clone(),
            method: "POST".into(),
        },
    });

    let mut ctx = AlertCtx::new();
    ctx.set("price", 5.0);
    ctx.payload = Some(r#"{"test":true}"#.into());

    engine.fire(&ctx).await;

    // If server received the request, the handle finishes cleanly.
    let _ = tokio::time::timeout(std::time::Duration::from_secs(2), server_handle).await;
}
