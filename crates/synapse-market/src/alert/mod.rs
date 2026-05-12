//! Alert rule-engine: PatternMatch / ThresholdCross → Webhook / Log dispatch.
//!
//! MQTT: live via `mqtt` feature flag (rumqttc = "0.24"). Best-effort publish, errors tolerated.

use std::collections::HashMap;

// ---------------------------------------------------------------------------
// Condition types
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, PartialEq)]
pub enum Op {
    Gt,
    Lt,
    Gte,
    Lte,
    Eq,
}

impl Op {
    fn eval(&self, lhs: f32, rhs: f32) -> bool {
        match self {
            Op::Gt  => lhs > rhs,
            Op::Lt  => lhs < rhs,
            Op::Gte => lhs >= rhs,
            Op::Lte => lhs <= rhs,
            Op::Eq  => (lhs - rhs).abs() < f32::EPSILON,
        }
    }
}

/// Column name (string key into AlertCtx::fields).
pub type Col = String;

#[derive(Debug, Clone)]
pub enum AlertCondition {
    PatternMatch { pattern_id: u64 },
    ThresholdCross { col: Col, op: Op, value: f32 },
    AndAll(Vec<AlertCondition>),
}

// ---------------------------------------------------------------------------
// Action types
// ---------------------------------------------------------------------------

#[derive(Debug, Clone)]
pub enum AlertAction {
    Webhook { url: String, method: String },
    /// MQTT publish via rumqttc (feature = "mqtt"). Best-effort, errors tolerated.
    Mqtt { broker: String, topic: String },
    Log,
}

// ---------------------------------------------------------------------------
// Rule
// ---------------------------------------------------------------------------

#[derive(Debug, Clone)]
pub struct AlertRule {
    pub id: String,
    pub condition: AlertCondition,
    pub action: AlertAction,
}

// ---------------------------------------------------------------------------
// Context
// ---------------------------------------------------------------------------

/// Runtime context passed to `AlertEngine::fire`.
#[derive(Debug, Default)]
pub struct AlertCtx {
    /// Numeric fields (e.g. "close" → 105.3).
    pub fields: HashMap<String, f32>,
    /// Set of matched pattern IDs.
    pub matched_patterns: Vec<u64>,
    /// Free-form payload to include in webhook body.
    pub payload: Option<String>,
}

impl AlertCtx {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn set(&mut self, col: &str, val: f32) -> &mut Self {
        self.fields.insert(col.to_string(), val);
        self
    }

    pub fn match_pattern(&mut self, id: u64) -> &mut Self {
        self.matched_patterns.push(id);
        self
    }
}

// ---------------------------------------------------------------------------
// Engine
// ---------------------------------------------------------------------------

pub struct AlertEngine {
    rules: Vec<AlertRule>,
    http: reqwest::Client,
}

impl AlertEngine {
    pub fn new() -> Self {
        Self {
            rules: Vec::new(),
            http: reqwest::Client::new(),
        }
    }

    pub fn register(&mut self, rule: AlertRule) {
        self.rules.push(rule);
    }

    pub async fn fire(&self, ctx: &AlertCtx) {
        for rule in &self.rules {
            if eval_condition(&rule.condition, ctx) {
                dispatch(&rule.action, &rule.id, ctx, &self.http).await;
            }
        }
    }
}

impl Default for AlertEngine {
    fn default() -> Self { Self::new() }
}

// ---------------------------------------------------------------------------
// Evaluation
// ---------------------------------------------------------------------------

fn eval_condition(cond: &AlertCondition, ctx: &AlertCtx) -> bool {
    match cond {
        AlertCondition::PatternMatch { pattern_id } => {
            ctx.matched_patterns.contains(pattern_id)
        }
        AlertCondition::ThresholdCross { col, op, value } => {
            ctx.fields.get(col).map(|&v| op.eval(v, *value)).unwrap_or(false)
        }
        AlertCondition::AndAll(conditions) => {
            conditions.iter().all(|c| eval_condition(c, ctx))
        }
    }
}

// ---------------------------------------------------------------------------
// Dispatch
// ---------------------------------------------------------------------------

async fn dispatch(action: &AlertAction, rule_id: &str, ctx: &AlertCtx, http: &reqwest::Client) {
    match action {
        AlertAction::Log => {
            let payload = ctx.payload.as_deref().unwrap_or("");
            tracing::info!(rule_id, payload, "alert fired");
            // Also print so tests/stdout can verify.
            println!("[alert] rule={rule_id} payload={payload}");
        }
        AlertAction::Webhook { url, method } => {
            let body = ctx.payload.clone().unwrap_or_else(|| {
                format!(r#"{{"rule":"{}","fields":{:?}}}"#, rule_id, ctx.fields)
            });
            let req = match method.to_uppercase().as_str() {
                "POST" => http.post(url).body(body).header("Content-Type", "application/json"),
                "PUT"  => http.put(url).body(body).header("Content-Type", "application/json"),
                _      => http.get(url),
            };
            match req.send().await {
                Ok(resp) => tracing::info!(rule_id, status = %resp.status(), "webhook ok"),
                Err(e)   => tracing::warn!(rule_id, error = %e, "webhook failed"),
            }
        }
        AlertAction::Mqtt { broker, topic } => {
            dispatch_mqtt(rule_id, broker, topic, ctx).await;
        }
    }
}

// ---------------------------------------------------------------------------
// MQTT dispatch (best-effort)
// ---------------------------------------------------------------------------

#[cfg(feature = "mqtt")]
async fn dispatch_mqtt(rule_id: &str, broker: &str, topic: &str, ctx: &AlertCtx) {
    use rumqttc::{AsyncClient, MqttOptions, QoS};

    // Parse "host:port" or default to port 1883
    let (host, port) = if let Some(pos) = broker.rfind(':') {
        let h = &broker[..pos];
        let p = broker[pos+1..].parse::<u16>().unwrap_or(1883);
        (h.to_owned(), p)
    } else {
        (broker.to_owned(), 1883u16)
    };

    let client_id = format!("synapse-market-alert-{rule_id}");
    let mut opts = MqttOptions::new(client_id, host, port);
    opts.set_keep_alive(std::time::Duration::from_secs(5));

    let (client, mut eventloop) = AsyncClient::new(opts, 4);

    let payload = ctx.payload.clone().unwrap_or_else(|| {
        format!(r#"{{"rule":"{rule_id}"}}"#)
    });

    // Spawn eventloop driver so the publish actually flushes
    tokio::spawn(async move {
        for _ in 0..20 {
            match eventloop.poll().await {
                Ok(_) => {}
                Err(_) => break,
            }
        }
    });

    match client.publish(topic, QoS::AtMostOnce, false, payload.as_bytes()).await {
        Ok(()) => tracing::info!(rule_id, topic, "MQTT publish queued"),
        Err(e) => tracing::warn!(rule_id, topic, error = %e, "MQTT publish failed (best-effort)"),
    }
    // Give the eventloop a moment to flush
    tokio::time::sleep(std::time::Duration::from_millis(50)).await;
}

#[cfg(not(feature = "mqtt"))]
async fn dispatch_mqtt(rule_id: &str, broker: &str, topic: &str, _ctx: &AlertCtx) {
    tracing::warn!(rule_id, broker, topic, "MQTT feature not enabled; rebuild with --features mqtt");
}
