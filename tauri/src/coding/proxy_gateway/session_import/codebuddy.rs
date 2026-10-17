//! CodeBuddy Code transcripts.
//!
//! The shape matches WorkBuddy (Claude-Code-style JSONL with the same three
//! usage sources), so the family parser does the reading. CodeBuddy adds one
//! rule of its own: its CLI accounts in Credits, and a non-zero Credit row is
//! real usage even when every token bucket is zero. `reported_cost_usd` is
//! therefore only set when the item carries an explicit USD amount; Credits are
//! not converted here because no exact price is known.

use super::parsers::ParsedSession;
use super::GatewayUsageTool;
use std::path::Path;

pub(super) fn parse(path: &Path, fallback: i64) -> Result<ParsedSession, String> {
    super::workbuddy_family::parse(GatewayUsageTool::Codebuddy, path, fallback)
}
