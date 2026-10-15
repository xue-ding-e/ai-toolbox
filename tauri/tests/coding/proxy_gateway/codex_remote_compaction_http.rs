//! Codex remote compaction v2 over the HTTP route.
//!
//! The unit tests around `compat::codex_remote_compaction` cover the pure
//! functions; these drive the real gateway so the *mounting* is covered too —
//! that a v2 turn is pulled out of the Responses identity passthrough, rewritten,
//! sealed, and replayed on the next turn. An earlier revision wired the module
//! correctly but only ever proved it by hand against a live upstream.

use super::support::*;
use ai_toolbox_lib::http_client;
use serde_json::{json, Value};
use std::time::Duration;
use tokio::net::{TcpListener, TcpStream};

const CAPSULE_PREFIX: &str = "ai-toolbox-compact-v1:";
const COMPACTED_CONTEXT_HEADING: &str = "Context summary from previous turns:";
const SUMMARY_TEXT: &str = "## Goal\nShip the compaction passthrough";

/// A Codex v2 compaction turn: an ordinary Responses request whose input ends in
/// `compaction_trigger`, carrying the tools and base instructions Codex sends.
fn compaction_turn(streaming: bool) -> Value {
    json!({
        "model": "fixture-model",
        "stream": streaming,
        "instructions": "You are a coding agent running in the Codex CLI.",
        "parallel_tool_calls": true,
        "tool_choice": "auto",
        "tools": [{
            "type": "function", "name": "read_file",
            "parameters": {"type": "object", "properties": {"path": {"type": "string"}}}
        }],
        "input": [
            { "type": "message", "role": "user", "content": [
                { "type": "input_text", "text": "compact the session" }
            ] },
            { "type": "compaction_trigger" }
        ]
    })
}

fn responses_reply(text: &str) -> Value {
    json!({
        "id": "upstream-summary", "object": "response", "status": "completed",
        "model": "fixture-model",
        "output": [{ "type": "message", "role": "assistant",
            "content": [{ "type": "output_text", "text": text }] }],
        "usage": { "input_tokens": 21, "output_tokens": 9, "total_tokens": 30 }
    })
}

async fn accept_upstream(listener: &TcpListener) -> TcpStream {
    tokio::time::timeout(Duration::from_secs(10), listener.accept())
        .await
        .expect("gateway must send the next upstream request")
        .unwrap()
        .0
}

async fn reply_with(socket: &mut TcpStream, reply: &Value) {
    write_mock_reply(socket, 200, &serde_json::to_vec(reply).unwrap(), false).await;
}

fn compaction_gateway(upstream_url: &str) -> RunningGateway {
    RunningGateway::new_for_api_format(
        upstream_url,
        "custom",
        BodyLogging::Disabled,
        "openai_responses",
    )
}

/// The same gateway pointed at a Chat-speaking upstream, the shape a relay
/// (sub2api and friends) presents when Codex's Responses turn is converted.
fn chat_compaction_gateway(upstream_url: &str) -> RunningGateway {
    RunningGateway::new_for_api_format(upstream_url, "custom", BodyLogging::Disabled, "openai_chat")
}

/// Serves the native attempt every Responses-target compaction turn makes
/// first, and returns the summarizing retry the Gateway sends once that attempt
/// is refused.
///
/// An upstream that does not implement v2 ignores the trigger and answers an
/// ordinary assistant message. That reply is discarded — Codex counts
/// `compaction` items, and an ordinary message counts zero of them — and the
/// turn is asked again as a plain summary. The first attempt is the price of
/// giving a native upstream its chance, so every "the Gateway sealed its own
/// item" test below starts from here.
///
/// The returned socket has already been fed the retry request; the caller
/// answers it.
async fn accept_a_refused_native_attempt(listener: &TcpListener) -> (TcpStream, Value) {
    let mut first = accept_upstream(listener).await;
    let body = read_mock_request(&mut first, "/v1/responses ").await;
    assert!(
        body["input"]
            .as_array()
            .unwrap()
            .iter()
            .any(|item| item["type"] == "compaction_trigger"),
        "the first attempt must offer the trigger: {body}"
    );
    reply_with(&mut first, &responses_reply("An ordinary assistant reply")).await;
    drop(first);

    let mut retry = accept_upstream(listener).await;
    let retry_body = read_mock_request(&mut retry, "/v1/responses ").await;
    assert!(
        !retry_body["input"]
            .as_array()
            .unwrap()
            .iter()
            .any(|item| item["type"] == "compaction_trigger"),
        "the retry must not offer the trigger again: {retry_body}"
    );
    (retry, retry_body)
}

/// A Chat upstream's streamed summary, which the gateway must aggregate back
/// into the Responses shape before it can be sealed.
fn chat_summary_stream(text: &str) -> Vec<u8> {
    let mut wire = String::new();
    for chunk in [
        json!({"id": "chatcmpl_compact_fixture", "object": "chat.completion.chunk",
               "model": "fixture-model",
               "choices": [{"index": 0, "delta": {"role": "assistant"}, "finish_reason": null}]}),
        json!({"id": "chatcmpl_compact_fixture", "object": "chat.completion.chunk",
               "model": "fixture-model",
               "choices": [{"index": 0, "delta": {"content": text}, "finish_reason": null}]}),
        json!({"id": "chatcmpl_compact_fixture", "object": "chat.completion.chunk",
               "model": "fixture-model",
               "choices": [{"index": 0, "delta": {}, "finish_reason": "stop"}],
               "usage": {"prompt_tokens": 21, "completion_tokens": 9, "total_tokens": 30}}),
    ] {
        wire.push_str("data: ");
        wire.push_str(&chunk.to_string());
        wire.push_str("\n\n");
    }
    wire.push_str("data: [DONE]\n\n");
    wire.into_bytes()
}

/// A v2 turn on the Responses identity route must leave it: the upstream sees a
/// plain summarization request, and the client gets a sealed compaction item
/// instead of the summary text.
#[tokio::test]
async fn identity_responses_compaction_is_rewritten_and_sealed() {
    let listener = TcpListener::bind(("127.0.0.1", 0)).await.unwrap();
    let upstream_url = format!("http://{}", listener.local_addr().unwrap());
    let upstream = tokio::spawn(async move {
        let (mut socket, body) = accept_a_refused_native_attempt(&listener).await;

        let input = body["input"].as_array().unwrap();
        assert!(
            !input.iter().any(|item| item["type"] == "compaction_trigger"),
            "the trigger leaked upstream: {body}"
        );
        assert_eq!(
            input.last().unwrap()["content"][0]["text"],
            "Produce the continuation summary now."
        );
        for stripped in ["tools", "tool_choice", "parallel_tool_calls"] {
            assert!(
                body.get(stripped).is_none(),
                "`{stripped}` leaked upstream: {body}"
            );
        }
        let instructions = body["instructions"].as_str().unwrap();
        assert!(
            instructions.contains("continuation summary"),
            "the summarization instruction did not replace Codex's: {instructions}"
        );
        assert!(
            !instructions.contains("Codex CLI"),
            "Codex's coding instructions leaked upstream: {instructions}"
        );
        assert_eq!(
            body["stream"],
            json!(false),
            "the summary must be fetched in one piece"
        );

        reply_with(&mut socket, &responses_reply(SUMMARY_TEXT)).await;
    });

    let gateway = compaction_gateway(&upstream_url);
    let client = http_client::create_client_no_proxy(10).unwrap();
    let response = client
        .post(&gateway.url)
        .json(&compaction_turn(true))
        .send()
        .await
        .unwrap();
    let status = response.status();
    let bytes = response.bytes().await.unwrap();
    assert_eq!(status, 200, "{}", String::from_utf8_lossy(&bytes));

    let events = sse_events(&bytes);
    assert_eq!(events.len(), 5, "{events:?}");
    let completed = completed_response(&events);
    let output = completed["output"].as_array().unwrap();
    assert_eq!(
        output.len(),
        1,
        "Codex installs the returned item as the whole history: {completed}"
    );
    assert_eq!(output[0]["type"], "compaction");
    assert!(
        output[0]["encrypted_content"]
            .as_str()
            .unwrap()
            .starts_with(CAPSULE_PREFIX),
        "{completed}"
    );
    assert_eq!(completed["usage"]["input_tokens"], 21);
    assert_eq!(completed["usage"]["output_tokens"], 9);
    assert!(
        !String::from_utf8_lossy(&bytes).contains("Ship the compaction passthrough"),
        "the summary reached the client in the clear: {}",
        String::from_utf8_lossy(&bytes)
    );

    upstream.await.unwrap();
}

/// A non-streaming client gets the same item as one JSON body. Codex always
/// streams, so this is the defensive branch, and it is the one that returns the
/// sealed item directly instead of re-rendering it as SSE.
#[tokio::test]
async fn a_non_streaming_client_still_gets_one_compaction_item() {
    let listener = TcpListener::bind(("127.0.0.1", 0)).await.unwrap();
    let upstream_url = format!("http://{}", listener.local_addr().unwrap());
    let upstream = tokio::spawn(async move {
        let (mut socket, _) = accept_a_refused_native_attempt(&listener).await;
        reply_with(&mut socket, &responses_reply(SUMMARY_TEXT)).await;
    });

    let gateway = compaction_gateway(&upstream_url);
    let client = http_client::create_client_no_proxy(10).unwrap();
    let response = client
        .post(&gateway.url)
        .json(&compaction_turn(false))
        .send()
        .await
        .unwrap();
    let status = response.status();
    let bytes = response.bytes().await.unwrap();
    assert_eq!(status, 200, "{}", String::from_utf8_lossy(&bytes));

    let body: Value = serde_json::from_slice(&bytes).unwrap();
    assert_eq!(body["status"], "completed");
    assert_eq!(body["output"].as_array().unwrap().len(), 1, "{body}");
    assert_eq!(body["output"][0]["type"], "compaction");
    assert!(
        body["output"][0]["encrypted_content"]
            .as_str()
            .unwrap()
            .starts_with(CAPSULE_PREFIX),
        "{body}"
    );
    assert!(
        !String::from_utf8_lossy(&bytes).contains("Ship the compaction passthrough"),
        "the summary reached the client in the clear: {body}"
    );

    upstream.await.unwrap();
}

/// The two axes are independent: this is the non-streaming caller of the pair
/// above, so the same buffered answer is rendered as JSON rather than SSE. It
/// enters aggregation through the *pre-existing* predicate (which declines
/// streaming clients), and still has to be sealed rather than passed through.
#[tokio::test]
async fn a_non_streaming_client_with_a_streaming_upstream_still_gets_one_item() {
    let listener = TcpListener::bind(("127.0.0.1", 0)).await.unwrap();
    let upstream_url = format!("http://{}", listener.local_addr().unwrap());
    let upstream = tokio::spawn(async move {
        let (mut socket, _) = accept_a_refused_native_attempt(&listener).await;
        let stream = format!(
            "event: response.output_item.done\ndata: {}\n\n\
             event: response.completed\ndata: {}\n\n",
            json!({"type": "response.output_item.done", "output_index": 0,
                   "item": {"type": "message", "role": "assistant",
                            "content": [{"type": "output_text", "text": SUMMARY_TEXT}]}}),
            json!({"type": "response.completed",
                   "response": responses_reply(SUMMARY_TEXT)})
        );
        write_mock_reply(&mut socket, 200, stream.as_bytes(), true).await;
    });

    let gateway = compaction_gateway(&upstream_url);
    let client = http_client::create_client_no_proxy(10).unwrap();
    let response = client
        .post(&gateway.url)
        .json(&compaction_turn(false))
        .send()
        .await
        .unwrap();
    let status = response.status();
    let bytes = response.bytes().await.unwrap();
    assert_eq!(status, 200, "{}", String::from_utf8_lossy(&bytes));

    // Not SSE: this caller did not ask for a stream.
    let body: Value = serde_json::from_slice(&bytes).unwrap();
    assert_eq!(body["status"], "completed");
    assert_eq!(body["output"].as_array().unwrap().len(), 1, "{body}");
    assert_eq!(
        body["output"][0]["type"], "compaction",
        "the summary was passed through instead of sealed: {body}"
    );
    assert!(
        body["output"][0]["encrypted_content"]
            .as_str()
            .unwrap()
            .starts_with(CAPSULE_PREFIX),
        "{body}"
    );
    assert_eq!(body["usage"]["input_tokens"], 21);
    assert!(
        !String::from_utf8_lossy(&bytes).contains("Ship the compaction passthrough"),
        "the summary reached the client in the clear: {body}"
    );

    upstream.await.unwrap();
}

/// The next turn replays the item Codex kept. The upstream must receive readable
/// context, not an opaque capsule it cannot open.
#[tokio::test]
async fn a_replayed_capsule_becomes_readable_context_upstream() {
    let listener = TcpListener::bind(("127.0.0.1", 0)).await.unwrap();
    let upstream_url = format!("http://{}", listener.local_addr().unwrap());
    let upstream = tokio::spawn(async move {
        let (mut socket, _) = accept_a_refused_native_attempt(&listener).await;
        reply_with(&mut socket, &responses_reply(SUMMARY_TEXT)).await;

        // The replay turn carries no trigger, so it is not a compaction turn at
        // all and reaches the upstream in one request.
        let mut socket = accept_upstream(&listener).await;
        let body = read_mock_request(&mut socket, "/v1/responses ").await;
        let input = body["input"].as_array().unwrap();
        assert_eq!(input[0]["type"], "message", "{body}");
        assert_eq!(input[0]["role"], "assistant", "{body}");
        let text = input[0]["content"][0]["text"].as_str().unwrap();
        assert!(
            text.starts_with(COMPACTED_CONTEXT_HEADING),
            "the summary must be labelled as background: {text}"
        );
        assert!(text.contains("Ship the compaction passthrough"), "{text}");
        assert!(
            !body.to_string().contains(CAPSULE_PREFIX),
            "the capsule reached an upstream that cannot open it: {body}"
        );
        assert!(
            body.get("tools").is_some(),
            "an ordinary turn keeps its tools: {body}"
        );
        assert!(
            !input.iter().any(|item| item["type"] == "compaction"),
            "the item must have been expanded, not passed through: {body}"
        );
        reply_with(&mut socket, &responses_reply("done")).await;
    });

    let gateway = compaction_gateway(&upstream_url);
    let client = http_client::create_client_no_proxy(10).unwrap();
    let first = client
        .post(&gateway.url)
        .json(&compaction_turn(true))
        .send()
        .await
        .unwrap();
    assert_eq!(first.status(), 200);
    let bytes = first.bytes().await.unwrap();
    let item = completed_response(&sse_events(&bytes))["output"][0].clone();
    assert_eq!(item["type"], "compaction");

    let mut replay = compaction_turn(false);
    replay["input"] = json!([
        item,
        { "type": "message", "role": "user", "content": [
            { "type": "input_text", "text": "keep going" }
        ] }
    ]);
    let second = client
        .post(&gateway.url)
        .json(&replay)
        .send()
        .await
        .unwrap();
    let status = second.status();
    let bytes = second.bytes().await.unwrap();
    assert_eq!(status, 200, "{}", String::from_utf8_lossy(&bytes));

    upstream.await.unwrap();
}

/// A session that compacts twice carries the first capsule *and* a fresh
/// trigger. Expanding the capsule is not optional then: a summarization turn
/// still travels to an upstream, and the Gateway's own ciphertext is one no
/// upstream can open — forwarding it is a guaranteed `invalid_encrypted_content`.
#[tokio::test]
async fn a_second_compaction_expands_the_previous_capsule_before_summarizing() {
    let listener = TcpListener::bind(("127.0.0.1", 0)).await.unwrap();
    let upstream_url = format!("http://{}", listener.local_addr().unwrap());
    let upstream = tokio::spawn(async move {
        let (mut socket, _) = accept_a_refused_native_attempt(&listener).await;
        reply_with(&mut socket, &responses_reply(SUMMARY_TEXT)).await;

        // The refusal above is remembered for this provider configuration, so
        // the second compaction goes straight to a summary rather than paying
        // for the same experiment twice.
        let mut socket = accept_upstream(&listener).await;
        let body = read_mock_request(&mut socket, "/v1/responses ").await;
        assert!(
            !body
                .get("input")
                .and_then(Value::as_array)
                .is_some_and(|input| input
                    .iter()
                    .any(|item| item["type"] == "compaction_trigger")),
            "the trigger must not be offered a second time: {body}"
        );
        // The second turn is still a summarization turn...
        assert_eq!(
            body["stream"],
            json!(false),
            "the summary must be fetched in one piece: {body}"
        );
        assert!(body.get("tools").is_none(), "tools leaked upstream: {body}");
        // ...but the item Codex kept from the first compaction is ours.
        assert!(
            !body.to_string().contains(CAPSULE_PREFIX),
            "the Gateway's own capsule reached an upstream that cannot open it: {body}"
        );
        assert!(
            !body["input"]
                .as_array()
                .unwrap()
                .iter()
                .any(|item| item["type"] == "compaction"),
            "the old item must be expanded, not passed through: {body}"
        );
        assert!(
            body.to_string().contains("Ship the compaction passthrough"),
            "the previous summary must reach the summarizer as context: {body}"
        );
        reply_with(&mut socket, &responses_reply(SUMMARY_TEXT)).await;
    });

    let gateway = compaction_gateway(&upstream_url);
    let client = http_client::create_client_no_proxy(10).unwrap();

    let first = client
        .post(&gateway.url)
        .json(&compaction_turn(true))
        .send()
        .await
        .unwrap();
    assert_eq!(first.status(), 200);
    let bytes = first.bytes().await.unwrap();
    let item = completed_response(&sse_events(&bytes))["output"][0].clone();
    assert_eq!(item["type"], "compaction");

    // Codex keeps the compaction item as the head of its history, so the next
    // compaction carries both it and a new `compaction_trigger`.
    let mut second_turn = compaction_turn(true);
    let mut input = second_turn["input"].as_array().unwrap().clone();
    input.insert(0, item);
    second_turn["input"] = json!(input);

    let second = client
        .post(&gateway.url)
        .json(&second_turn)
        .send()
        .await
        .unwrap();
    let status = second.status();
    let bytes = second.bytes().await.unwrap();
    assert_eq!(status, 200, "{}", String::from_utf8_lossy(&bytes));

    upstream.await.unwrap();
}

/// An unusable summary must fail the turn. Codex installs whatever item comes
/// back as the entire conversation history, so a placeholder would silently
/// discard the context the compaction was supposed to preserve.
#[tokio::test]
async fn an_empty_summary_fails_instead_of_faking_an_item() {
    let listener = TcpListener::bind(("127.0.0.1", 0)).await.unwrap();
    let upstream_url = format!("http://{}", listener.local_addr().unwrap());
    let upstream = tokio::spawn(async move {
        let (mut socket, _) = accept_a_refused_native_attempt(&listener).await;
        // 200, but nothing to summarize with.
        reply_with(&mut socket, &responses_reply("   ")).await;
    });

    let gateway = compaction_gateway(&upstream_url);
    let client = http_client::create_client_no_proxy(10).unwrap();
    let response = client
        .post(&gateway.url)
        .json(&compaction_turn(true))
        .send()
        .await
        .unwrap();
    let status = response.status();
    let bytes = response.bytes().await.unwrap();
    // 502, not 400: nothing is wrong with Codex's request, and an upstream-class
    // error is what keeps the turn retryable instead of blaming the client.
    assert_eq!(status, 502, "{}", String::from_utf8_lossy(&bytes));

    let body: Value = serde_json::from_slice(&bytes).unwrap();
    assert_eq!(body["error"], "upstream_forward_failed");
    assert!(
        body["message"]
            .as_str()
            .unwrap()
            .contains("empty compaction summary"),
        "{body}"
    );
    assert!(body.get("output").is_none(), "an item was faked: {body}");
    assert!(
        !String::from_utf8_lossy(&bytes).contains(CAPSULE_PREFIX),
        "a capsule was sealed from nothing: {body}"
    );

    upstream.await.unwrap();
}

/// A truncated summary must fail the turn too - even though the upstream already
/// produced text. Codex installs the returned item as the entire conversation
/// history, so sealing a half-built summary would silently drop the context the
/// compaction exists to preserve.
///
/// This is the shape issue #416 reported as `Upstream returned an empty
/// compaction summary`: the answer was truncated, not empty, and the
/// classification only looked at Chat / Anthropic stop reasons - which the
/// converted Responses body never carries. The gateway asks for `stream: false`
/// and a relay answers SSE anyway, which is how a truncation reaches an
/// aggregating gateway in the first place.
#[tokio::test]
async fn a_truncated_summary_fails_instead_of_sealing_partial_context() {
    let listener = TcpListener::bind(("127.0.0.1", 0)).await.unwrap();
    let upstream_url = format!("http://{}", listener.local_addr().unwrap());
    let upstream = tokio::spawn(async move {
        let (mut socket, _) = accept_a_refused_native_attempt(&listener).await;
        // A truncated turn: text arrived, then the upstream said it stopped.
        let stream = format!(
            "event: response.output_text.delta\ndata: {}\n\n\
             event: response.incomplete\ndata: {}\n\n",
            json!({"type": "response.output_text.delta", "delta": SUMMARY_TEXT}),
            json!({"type": "response.incomplete",
                   "response": {"id": "upstream-summary", "object": "response",
                                "status": "incomplete",
                                "incomplete_details": {"reason": "max_output_tokens"},
                                "model": "fixture-model",
                                "output": [{"type": "message", "role": "assistant",
                                            "content": [{"type": "output_text", "text": SUMMARY_TEXT}]}],
                                "usage": {"input_tokens": 21, "output_tokens": 9, "total_tokens": 30}}})
        );
        write_mock_reply(&mut socket, 200, stream.as_bytes(), true).await;
    });

    let gateway = compaction_gateway(&upstream_url);
    let client = http_client::create_client_no_proxy(10).unwrap();
    let response = client
        .post(&gateway.url)
        .json(&compaction_turn(true))
        .send()
        .await
        .unwrap();
    let status = response.status();
    let bytes = response.bytes().await.unwrap();
    assert_eq!(status, 502, "{}", String::from_utf8_lossy(&bytes));

    let body: Value = serde_json::from_slice(&bytes).unwrap();
    assert_eq!(body["error"], "upstream_forward_failed");
    assert!(
        body["message"]
            .as_str()
            .unwrap()
            .contains("truncated the compaction summary"),
        "{body}"
    );
    assert!(body.get("output").is_none(), "an item was faked: {body}");
    assert!(
        !String::from_utf8_lossy(&bytes).contains(CAPSULE_PREFIX),
        "a partial summary was sealed: {body}"
    );

    upstream.await.unwrap();
}

/// A relay that ignores the `stream: false` the gateway asks for must still
/// produce the compaction contract: the SSE answer is buffered whole, sealed
/// into one `compaction` item, and re-rendered for the streaming client.
///
/// Relaying that SSE instead would hand Codex an ordinary assistant message,
/// which counts zero `compaction` items — the `got 0 from 1 output items` the
/// upstream's own summary triggers. Codex always streams its compaction turn,
/// so `request_declares_streaming` must not exempt it from aggregation.
#[tokio::test]
async fn a_streaming_upstream_still_yields_one_compaction_item() {
    let listener = TcpListener::bind(("127.0.0.1", 0)).await.unwrap();
    let upstream_url = format!("http://{}", listener.local_addr().unwrap());
    let upstream = tokio::spawn(async move {
        let (mut socket, _) = accept_a_refused_native_attempt(&listener).await;
        // The upstream answers SSE even though the request asked for one piece.
        let stream = format!(
            "event: response.output_item.done\ndata: {}\n\n\
             event: response.completed\ndata: {}\n\n",
            json!({"type": "response.output_item.done", "output_index": 0,
                   "item": {"type": "message", "role": "assistant",
                            "content": [{"type": "output_text", "text": SUMMARY_TEXT}]}}),
            json!({"type": "response.completed",
                   "response": responses_reply(SUMMARY_TEXT)})
        );
        write_mock_reply(&mut socket, 200, stream.as_bytes(), true).await;
    });

    let gateway = compaction_gateway(&upstream_url);
    let client = http_client::create_client_no_proxy(10).unwrap();
    let response = client
        .post(&gateway.url)
        .json(&compaction_turn(true))
        .send()
        .await
        .unwrap();
    let status = response.status();
    let bytes = response.bytes().await.unwrap();
    assert_eq!(status, 200, "{}", String::from_utf8_lossy(&bytes));

    let events = sse_events(&bytes);
    let completed = completed_response(&events);
    let output = completed["output"].as_array().unwrap();
    assert_eq!(
        output.len(),
        1,
        "Codex installs the returned item as the whole history: {completed}"
    );
    assert_eq!(
        output[0]["type"], "compaction",
        "a plain message item satisfies zero compaction items: {completed}"
    );
    assert!(
        output[0]["encrypted_content"]
            .as_str()
            .unwrap()
            .starts_with(CAPSULE_PREFIX),
        "{completed}"
    );
    assert_eq!(completed["usage"]["input_tokens"], 21);
    assert_eq!(completed["usage"]["output_tokens"], 9);
    assert!(
        !String::from_utf8_lossy(&bytes).contains("Ship the compaction passthrough"),
        "the summary reached the client in the clear: {}",
        String::from_utf8_lossy(&bytes)
    );

    upstream.await.unwrap();
}

/// The same buffering on a Chat-speaking upstream, where the stream has to be
/// aggregated *and* reverse-converted before it is in any shape to seal. This
/// is the relay arrangement the failure was reported against.
#[tokio::test]
async fn a_streaming_chat_upstream_still_yields_one_compaction_item() {
    let listener = TcpListener::bind(("127.0.0.1", 0)).await.unwrap();
    let upstream_url = format!("http://{}", listener.local_addr().unwrap());
    let upstream = tokio::spawn(async move {
        let mut socket = accept_upstream(&listener).await;
        read_mock_request(&mut socket, "/v1/chat/completions ").await;
        write_mock_reply(&mut socket, 200, &chat_summary_stream(SUMMARY_TEXT), true).await;
    });

    let gateway = chat_compaction_gateway(&upstream_url);
    let client = http_client::create_client_no_proxy(10).unwrap();
    let response = client
        .post(&gateway.url)
        .json(&compaction_turn(true))
        .send()
        .await
        .unwrap();
    let status = response.status();
    let bytes = response.bytes().await.unwrap();
    assert_eq!(status, 200, "{}", String::from_utf8_lossy(&bytes));

    let events = sse_events(&bytes);
    let completed = completed_response(&events);
    let output = completed["output"].as_array().unwrap();
    assert_eq!(output.len(), 1, "expected one item: {completed}");
    assert_eq!(
        output[0]["type"], "compaction",
        "the Chat summary was relayed instead of sealed: {completed}"
    );
    assert!(
        output[0]["encrypted_content"]
            .as_str()
            .unwrap()
            .starts_with(CAPSULE_PREFIX),
        "{completed}"
    );
    assert_eq!(completed["usage"]["input_tokens"], 21);
    assert_eq!(completed["usage"]["output_tokens"], 9);
    assert!(
        !String::from_utf8_lossy(&bytes).contains("Ship the compaction passthrough"),
        "the summary reached the client in the clear: {}",
        String::from_utf8_lossy(&bytes)
    );

    upstream.await.unwrap();
}

/// An ordinary Responses SSE answer carrying one `compaction` item — what an
/// upstream that really implements v2 returns.
fn native_compaction_stream() -> Vec<u8> {
    let item = json!({
        "id": "cmp_upstream_1", "type": "compaction",
        "encrypted_content": "gAAAAABq-upstream-cipher"
    });
    let completed = json!({
        "id": "resp_native", "object": "response", "status": "completed",
        "model": "fixture-model", "output": [item],
        "usage": {"input_tokens": 40, "output_tokens": 5, "total_tokens": 45}
    });
    responses_event_stream(&[
        json!({"type": "response.created", "response": {"id": "resp_native", "output": []}}),
        json!({"type": "response.output_item.done", "output_index": 0, "item": item}),
        json!({"type": "response.completed", "response": completed}),
    ])
}

fn responses_event_stream(events: &[Value]) -> Vec<u8> {
    let mut wire = String::new();
    for event in events {
        wire.push_str("event: ");
        wire.push_str(event["type"].as_str().unwrap());
        wire.push_str("\ndata: ");
        wire.push_str(&event.to_string());
        wire.push_str("\n\n");
    }
    wire.into_bytes()
}

/// Asserts that no further upstream request follows, which is how "this turn
/// needed no retry" is observable from the fixture side. The listener outlives
/// the request that was just answered, so a stray retry would be accepted rather
/// than refused — and would have to be answered by a mock that has already
/// returned.
async fn assert_no_further_upstream_request(listener: &TcpListener, why: &str) {
    assert!(
        tokio::time::timeout(Duration::from_millis(500), listener.accept())
            .await
            .is_err(),
        "{why}"
    );
}

/// A Responses target is offered the trigger first and, when the upstream really
/// implements v2, its own item is relayed untouched — no summarizing request, and
/// the upstream's ciphertext rather than this Gateway's capsule.
///
/// This is the whole point of trying the native path: a user behind a chain that
/// ends at such an upstream gets what Codex's own upstream would have given.
#[tokio::test]
async fn a_responses_target_relays_the_upstreams_own_item() {
    let listener = TcpListener::bind(("127.0.0.1", 0)).await.unwrap();
    let upstream_url = format!("http://{}", listener.local_addr().unwrap());
    let upstream = tokio::spawn(async move {
        let mut socket = accept_upstream(&listener).await;
        let body = read_mock_request(&mut socket, "/v1/responses ").await;

        assert!(
            body["input"]
                .as_array()
                .unwrap()
                .iter()
                .any(|item| item["type"] == "compaction_trigger"),
            "the trigger must reach a native upstream: {body}"
        );
        assert_eq!(
            body["instructions"], "You are a coding agent running in the Codex CLI.",
            "the coding instructions must survive a pass-through turn: {body}"
        );
        assert!(body.get("tools").is_some(), "tools should survive too: {body}");
        assert_eq!(
            body["stream"],
            json!(true),
            "the turn reaches the upstream exactly as Codex wrote it: {body}"
        );

        write_mock_reply(&mut socket, 200, &native_compaction_stream(), true).await;
        assert_no_further_upstream_request(
            &listener,
            "a native reply must not be followed by a summarizing retry",
        )
        .await;
    });

    let gateway = compaction_gateway(&upstream_url);
    let client = http_client::create_client_no_proxy(10).unwrap();
    let response = client
        .post(&gateway.url)
        .json(&compaction_turn(true))
        .send()
        .await
        .unwrap();
    let status = response.status();
    let bytes = response.bytes().await.unwrap();
    assert_eq!(status, 200, "{}", String::from_utf8_lossy(&bytes));

    let events = sse_events(&bytes);
    let completed = completed_response(&events);
    let output = completed["output"].as_array().unwrap();
    assert_eq!(output.len(), 1, "{completed}");
    assert_eq!(output[0]["type"], "compaction");
    assert_eq!(
        output[0]["encrypted_content"], "gAAAAABq-upstream-cipher",
        "the upstream's own ciphertext must be relayed, not replaced: {completed}"
    );
    assert_eq!(completed["usage"]["input_tokens"], 40);
    assert_eq!(completed["usage"]["output_tokens"], 5);
    // The lifecycle is the upstream's own, replayed as it streamed.
    assert_eq!(
        events
            .iter()
            .filter(|event| event["type"] == "response.completed")
            .count(),
        1,
        "exactly one terminal must reach Codex: {events:?}"
    );
    assert!(
        !String::from_utf8_lossy(&bytes).contains(CAPSULE_PREFIX),
        "a native turn must not be sealed by the Gateway: {completed}"
    );

    upstream.await.unwrap();
}

/// A relay that answers with an ordinary assistant message instead of a
/// `compaction` item is refused once and asked again as a summary. The item the
/// client finally gets is the Gateway's own, built from a turn that really was
/// written under the summarization instruction.
#[tokio::test]
async fn a_reply_without_a_compaction_item_is_retried_as_a_summary() {
    let listener = TcpListener::bind(("127.0.0.1", 0)).await.unwrap();
    let upstream_url = format!("http://{}", listener.local_addr().unwrap());
    let upstream = tokio::spawn(async move {
        let (mut retry, body) = accept_a_refused_native_attempt(&listener).await;
        assert_eq!(
            body["stream"],
            json!(false),
            "the summary must be fetched in one piece: {body}"
        );
        assert!(body.get("tools").is_none(), "tools leaked upstream: {body}");
        assert!(
            body["instructions"]
                .as_str()
                .unwrap()
                .contains("continuation summary"),
            "the summarization instruction did not replace Codex's: {body}"
        );
        reply_with(&mut retry, &responses_reply(SUMMARY_TEXT)).await;

        assert_no_further_upstream_request(&listener, "a compaction turn is retried at most once")
            .await;
    });

    let gateway = compaction_gateway(&upstream_url);
    let client = http_client::create_client_no_proxy(10).unwrap();
    let response = client
        .post(&gateway.url)
        .json(&compaction_turn(true))
        .send()
        .await
        .unwrap();
    let status = response.status();
    let bytes = response.bytes().await.unwrap();
    assert_eq!(status, 200, "{}", String::from_utf8_lossy(&bytes));

    let events = sse_events(&bytes);
    let completed = completed_response(&events);
    let output = completed["output"].as_array().unwrap();
    assert_eq!(
        output.len(),
        1,
        "the ordinary assistant reply reached Codex: {completed}"
    );
    assert_eq!(output[0]["type"], "compaction");
    assert!(
        output[0]["encrypted_content"]
            .as_str()
            .unwrap()
            .starts_with(CAPSULE_PREFIX),
        "the fallback must seal its own item: {completed}"
    );
    assert!(
        !String::from_utf8_lossy(&bytes).contains("An ordinary assistant reply"),
        "the refused reply leaked to the client: {}",
        String::from_utf8_lossy(&bytes)
    );
    assert!(
        !String::from_utf8_lossy(&bytes).contains("Ship the compaction passthrough"),
        "the summary reached the client in the clear: {}",
        String::from_utf8_lossy(&bytes)
    );

    upstream.await.unwrap();
}

/// An upstream that rejects the unknown input item outright is saying the same
/// thing a silent relay says — "I do not understand this trigger" — so it earns
/// the same single retry instead of failing the turn.
#[tokio::test]
async fn an_upstream_that_rejects_the_trigger_is_retried_as_a_summary() {
    let listener = TcpListener::bind(("127.0.0.1", 0)).await.unwrap();
    let upstream_url = format!("http://{}", listener.local_addr().unwrap());
    let upstream = tokio::spawn(async move {
        let mut first = accept_upstream(&listener).await;
        let body = read_mock_request(&mut first, "/v1/responses ").await;
        assert!(
            body["input"]
                .as_array()
                .unwrap()
                .iter()
                .any(|item| item["type"] == "compaction_trigger"),
            "the first attempt must offer the trigger: {body}"
        );
        write_mock_reply(
            &mut first,
            400,
            br#"{"error":{"message":"unknown input item type: compaction_trigger"}}"#,
            false,
        )
        .await;
        drop(first);

        let mut retry = accept_upstream(&listener).await;
        let retry_body = read_mock_request(&mut retry, "/v1/responses ").await;
        assert!(
            retry_body["input"]
                .as_array()
                .unwrap()
                .iter()
                .all(|item| item["type"] != "compaction_trigger"),
            "the retry must not resend the rejected item: {retry_body}"
        );
        reply_with(&mut retry, &responses_reply(SUMMARY_TEXT)).await;
    });

    let gateway = compaction_gateway(&upstream_url);
    let client = http_client::create_client_no_proxy(10).unwrap();
    let response = client
        .post(&gateway.url)
        .json(&compaction_turn(true))
        .send()
        .await
        .unwrap();
    let status = response.status();
    let bytes = response.bytes().await.unwrap();
    assert_eq!(
        status, 200,
        "a rejected trigger must not fail the turn: {}",
        String::from_utf8_lossy(&bytes)
    );

    let completed = completed_response(&sse_events(&bytes));
    assert_eq!(completed["output"][0]["type"], "compaction", "{completed}");
    assert!(
        completed["output"][0]["encrypted_content"]
            .as_str()
            .unwrap()
            .starts_with(CAPSULE_PREFIX),
        "{completed}"
    );

    upstream.await.unwrap();
}

/// A rate limit is not an answer about the trigger, so the turn is not resent as
/// a summary: that would spend a second, equally rate-limited request and hide
/// the real error behind it.
#[tokio::test]
async fn a_rate_limited_attempt_is_not_retried() {
    let listener = TcpListener::bind(("127.0.0.1", 0)).await.unwrap();
    let upstream_url = format!("http://{}", listener.local_addr().unwrap());
    let upstream = tokio::spawn(async move {
        let mut socket = accept_upstream(&listener).await;
        read_mock_request(&mut socket, "/v1/responses ").await;
        write_mock_reply(
            &mut socket,
            429,
            br#"{"error":{"message":"rate limit reached"}}"#,
            false,
        )
        .await;
        drop(socket);

        assert_no_further_upstream_request(
            &listener,
            "a rate limit must not be retried with a different body",
        )
        .await;
    });

    let gateway = compaction_gateway(&upstream_url);
    let client = http_client::create_client_no_proxy(10).unwrap();
    let response = client
        .post(&gateway.url)
        .json(&compaction_turn(true))
        .send()
        .await
        .unwrap();
    assert_eq!(
        response.status(),
        429,
        "the upstream's own status must surface"
    );

    upstream.await.unwrap();
}

/// The refused attempt is remembered for this provider configuration, so the
/// second compaction in a session goes straight to a summary. Without it every
/// compaction would carry the whole conversation to the upstream twice.
///
/// The memory is volatile: a Gateway restart — or any change to the provider's
/// identity, which the key is derived from — offers the trigger again.
#[tokio::test]
async fn a_refused_native_attempt_is_not_offered_again() {
    let listener = TcpListener::bind(("127.0.0.1", 0)).await.unwrap();
    let upstream_url = format!("http://{}", listener.local_addr().unwrap());
    let upstream = tokio::spawn(async move {
        let (mut retry, _) = accept_a_refused_native_attempt(&listener).await;
        reply_with(&mut retry, &responses_reply(SUMMARY_TEXT)).await;

        // The second compaction of this session: one request, already a summary.
        let mut socket = accept_upstream(&listener).await;
        let body = read_mock_request(&mut socket, "/v1/responses ").await;
        assert!(
            body["input"]
                .as_array()
                .unwrap()
                .iter()
                .all(|item| item["type"] != "compaction_trigger"),
            "the trigger was offered a second time: {body}"
        );
        reply_with(&mut socket, &responses_reply(SUMMARY_TEXT)).await;
    });

    let gateway = compaction_gateway(&upstream_url);
    let client = http_client::create_client_no_proxy(10).unwrap();
    for _ in 0..2 {
        let response = client
            .post(&gateway.url)
            .json(&compaction_turn(true))
            .send()
            .await
            .unwrap();
        let status = response.status();
        let bytes = response.bytes().await.unwrap();
        assert_eq!(status, 200, "{}", String::from_utf8_lossy(&bytes));
        let completed = completed_response(&sse_events(&bytes));
        assert_eq!(
            completed["output"].as_array().unwrap().len(),
            1,
            "{completed}"
        );
    }

    upstream.await.unwrap();
}
