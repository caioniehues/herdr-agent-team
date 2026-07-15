//! Schema-gated, display-only metadata for team workers.

use serde::{Deserialize, Serialize};
use serde_json::Value;

pub const SOURCE: &str = "caioniehues:herdr-agent-team";

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct MetadataCapabilities {
    pub report_metadata: bool,
    pub title: bool,
    pub display_agent: bool,
    pub custom_status: bool,
    pub state_labels: bool,
    pub seq: bool,
    pub ttl_ms: bool,
}

impl MetadataCapabilities {
    pub fn from_schema(schema: &str) -> Self {
        let Ok(schema) = serde_json::from_str::<Value>(schema) else {
            return Self::default();
        };
        let Some(params) = schema
            .pointer("/schemas/request/$defs/PaneReportMetadataParams")
            .and_then(Value::as_object)
        else {
            return Self::default();
        };
        let properties = params.get("properties").and_then(Value::as_object);
        let has = |name| properties.is_some_and(|properties| properties.contains_key(name));
        Self {
            report_metadata: has("pane_id") && has("source"),
            title: has("title"),
            display_agent: has("display_agent"),
            custom_status: has("custom_status"),
            state_labels: has("state_labels"),
            seq: has("seq"),
            ttl_ms: has("ttl_ms"),
        }
    }

    pub fn can_publish(&self) -> bool {
        self.report_metadata && (self.title || self.display_agent)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MetadataFacts<'a> {
    pub team: &'a str,
    pub role: &'a str,
    pub task: Option<&'a str>,
    pub status: &'a str,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MetadataUpdate {
    pub title: Option<String>,
    pub display_agent: Option<String>,
    pub custom_status: Option<String>,
    pub state_label: Option<(String, String)>,
    pub seq: Option<u64>,
    pub ttl_ms: Option<u64>,
}

/// Maps team-domain facts to the tokens exposed by the installed Herdr schema.
pub fn map_facts(
    facts: MetadataFacts<'_>,
    capabilities: &MetadataCapabilities,
    sequence: u64,
) -> Option<MetadataUpdate> {
    if !capabilities.can_publish() {
        return None;
    }
    let compact = format!("{}/{}", facts.team, facts.role);
    Some(MetadataUpdate {
        title: capabilities
            .title
            .then(|| facts.task.map(str::to_owned))
            .flatten(),
        display_agent: capabilities.display_agent.then(|| compact.clone()),
        custom_status: capabilities.custom_status.then_some(compact.clone()),
        state_label: capabilities
            .state_labels
            .then(|| (facts.status.to_owned(), compact)),
        seq: capabilities.seq.then_some(sequence),
        // An explicit attention request is a transient presentation ping.
        ttl_ms: (facts.status == "needs_attention" && capabilities.ttl_ms).then_some(30_000),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn schema_gate_falls_back_when_custom_tokens_are_absent() {
        let schema = r#"{"schemas":{"request":{"$defs":{"PaneReportMetadataParams":{"properties":{"pane_id":{},"source":{},"title":{},"display_agent":{}}}}}}}"#;
        let capabilities = MetadataCapabilities::from_schema(schema);
        let update = map_facts(
            MetadataFacts {
                team: "wave3",
                role: "builder",
                task: Some("implement gate"),
                status: "working",
            },
            &capabilities,
            3,
        )
        .expect("fallback remains publishable");
        assert_eq!(update.title.as_deref(), Some("implement gate"));
        assert_eq!(update.display_agent.as_deref(), Some("wave3/builder"));
        assert_eq!(update.custom_status, None);
        assert_eq!(update.state_label, None);
        assert_eq!(update.seq, None);
    }
}
