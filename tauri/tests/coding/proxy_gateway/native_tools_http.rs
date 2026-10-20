//! Native-tool and rewrite markers over the real HTTP route.
//!
//! The unit tests in `runtime::observations` cover the pure scanners. These
//! drive the gateway end to end to prove the markers are *mounted*: declared
//! from the client body, forwarded from the body actually sent upstream, used
//! from the upstream reply, and persisted in the JSONL detail (never in the
//! SQLite summary, which is what the request list reads).

use super::support::*;
use ai_toolbox_lib::coding::proxy_gateway::request_log;
use ai_toolbox_lib::coding::proxy_gateway::types::{
    GatewayRequestLogDetail, ProxyGatewayRequestLogListInput,
};
use ai_toolbox_lib::http_client;
use serde_json::{json, Value};
use std::time::Duration;
use tokio::net::{TcpListener, TcpStream};

async fn accept_upstream(listener: &TcpListener) -> TcpStream {
    tokio::time::timeout(Duration::from_secs(10), listener.accept())
        .await
        .expect("gateway must send an upstream request")
        .unwrap()
        .0
}

fn responses_request(tools: Value) -> Value {
    json!({
        "model": "fixture-model",
        "stream": false,
        "input": [
            { "type": "message", "role": "user", "content": [
                { "type": "input_text", "text": "search for the answer" }
            ] }
        ],
        "tools": tools
    })
}

fn responses_search_reply() -> Value {
    json!({
        "id": "resp_search", "object": "response", "status": "completed",
        "model": "fixture-model",
        "output": [
            { "type": "web_search_call", "id": "ws_1", "status": "completed" },
            { "type": "message", "role": "assistant", "content": [
                { "type": "output_text", "text": "found it" }
            ] }
        ],
        "usage": { "input_tokens": 11, "output_tokens": 4, "total_tokens": 15 }
    })
}

fn chat_reply() -> Vec<u8> {
    serde_json::to_vec(&json!({
        "id": "chatcmpl_fixture", "object": "chat.completion", "model": "fixture-model",
        "choices": [{ "index": 0, "message": { "role": "assistant", "content": "found it" },
                      "finish_reason": "stop" }],
        "usage": { "prompt_tokens": 7, "completion_tokens": 2, "total_tokens": 9 }
    }))
    .unwrap()
}

/// Observability is recorded after the client response is written, so the
/// JSONL detail appears asynchronously. Poll instead of assuming ordering.
fn only_recorded_detail(gateway: &RunningGateway) -> GatewayRequestLogDetail {
    for _ in 0..100 {
        let summaries = request_log::list_request_logs(
            &gateway.paths(),
            ProxyGatewayRequestLogListInput { limit: Some(10) },
        )
        .expect("list request logs");
        if let Some(trace_id) = summaries.first().map(|summary| summary.trace_id.clone()) {
            if let Some(detail) = request_log::get_request_log_detail(&gateway.paths(), &trace_id)
                .expect("read request log")
            {
                return detail;
            }
        }
        std::thread::sleep(std::time::Duration::from_millis(20));
    }
    panic!("one recorded request with a JSONL detail");
}

#[tokio::test]
async fn a_native_search_is_marked_declared_forwarded_and_used() {
    let listener = TcpListener::bind(("127.0.0.1", 0)).await.unwrap();
    let upstream_url = format!("http://{}", listener.local_addr().unwrap());
    let gateway = RunningGateway::new_for_api_format(
        &upstream_url,
        "custom",
        BodyLogging::Enabled,
        "openai_responses",
    );

    let upstream = tokio::spawn(async move {
        let mut socket = accept_upstream(&listener).await;
        let forwarded = read_mock_request(&mut socket, "/v1/responses").await;
        assert_eq!(
            forwarded["tools"][0]["type"], "web_search",
            "the declaration must reach the upstream unchanged: {forwarded}"
        );
        write_mock_reply(
            &mut socket,
            200,
            &serde_json::to_vec(&responses_search_reply()).unwrap(),
            false,
        )
        .await;
    });

    let client = http_client::create_client_no_proxy(10).unwrap();
    let response = client
        .post(&gateway.url)
        .json(&responses_request(json!([{ "type": "web_search" }])))
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), 200);
    upstream.await.unwrap();

    let detail = only_recorded_detail(&gateway);
    let search = detail
        .native_tools
        .iter()
        .find(|tool| tool.kind == "web_search")
        .expect("web search marker");
    assert!(search.declared);
    assert!(search.forwarded);
    assert!(search.used);
    assert!(
        !detail
            .rewrites
            .iter()
            .any(|rewrite| rewrite.kind == "native_tool_filtered"),
        "nothing was filtered, so no filter marker: {:?}",
        detail.rewrites
    );
}

#[tokio::test]
async fn a_tool_dropped_by_protocol_conversion_is_marked_filtered() {
    let listener = TcpListener::bind(("127.0.0.1", 0)).await.unwrap();
    let upstream_url = format!("http://{}", listener.local_addr().unwrap());
    // Responses source → Chat target: the hosted tool has no Chat representation
    // and the outbound adapter filters it.
    let gateway = RunningGateway::new_for_api_format(
        &upstream_url,
        "custom",
        BodyLogging::Enabled,
        "openai_chat",
    );

    let upstream = tokio::spawn(async move {
        let mut socket = accept_upstream(&listener).await;
        let forwarded = read_mock_request(&mut socket, "/v1/chat/completions").await;
        let forwarded_tools = forwarded["tools"].as_array().cloned().unwrap_or_default();
        assert!(
            forwarded_tools
                .iter()
                .all(|tool| tool["type"].as_str() != Some("web_search")),
            "the hosted tool must not survive into a Chat body: {forwarded}"
        );
        write_mock_reply(&mut socket, 200, &chat_reply(), false).await;
    });

    let client = http_client::create_client_no_proxy(10).unwrap();
    let response = client
        .post(&gateway.url)
        .json(&responses_request(json!([
            { "type": "function", "name": "shell", "parameters": {"type": "object", "properties": {}} },
            { "type": "web_search" }
        ])))
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), 200);
    upstream.await.unwrap();

    let detail = only_recorded_detail(&gateway);
    let search = detail
        .native_tools
        .iter()
        .find(|tool| tool.kind == "web_search")
        .expect("web search marker");
    assert!(search.declared);
    assert!(!search.forwarded, "the declaration was dropped");
    assert!(
        detail.rewrites.iter().any(|rewrite| {
            rewrite.kind == "native_tool_filtered"
                && rewrite.detail.as_deref() == Some("web_search")
        }),
        "the filter must leave a marker: {:?}",
        detail.rewrites
    );
}

#[tokio::test]
async fn detail_only_markers_do_not_exist_without_request_logging() {
    let listener = TcpListener::bind(("127.0.0.1", 0)).await.unwrap();
    let upstream_url = format!("http://{}", listener.local_addr().unwrap());
    // Request logging off: the markers live only in the JSONL detail, so no
    // detail file may be written at all (and the list stays empty).
    let gateway = RunningGateway::new_for_api_format(
        &upstream_url,
        "custom",
        BodyLogging::Disabled,
        "openai_responses",
    );

    let upstream = tokio::spawn(async move {
        let mut socket = accept_upstream(&listener).await;
        let _ = read_mock_request(&mut socket, "/v1/responses").await;
        write_mock_reply(
            &mut socket,
            200,
            &serde_json::to_vec(&responses_search_reply()).unwrap(),
            false,
        )
        .await;
    });

    let client = http_client::create_client_no_proxy(10).unwrap();
    let response = client
        .post(&gateway.url)
        .json(&responses_request(json!([{ "type": "web_search" }])))
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), 200);
    upstream.await.unwrap();

    assert!(
        request_log::list_request_logs(
            &gateway.paths(),
            ProxyGatewayRequestLogListInput { limit: Some(10) },
        )
        .unwrap()
        .is_empty(),
        "request logging is off, so no JSONL detail exists"
    );
}
