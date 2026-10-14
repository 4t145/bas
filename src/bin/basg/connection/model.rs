use super::{Condition, Failure};
use bas::{authentication::elicitation, service::pb};

#[derive(Clone, Debug, PartialEq)]
pub(crate) struct AgentSummary {
    pub id: String,
    pub kind: String,
    pub command: String,
}

impl From<pb::Agent> for AgentSummary {
    fn from(agent: pb::Agent) -> Self {
        Self {
            id: agent.id,
            kind: agent.kind,
            command: agent.command,
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub(crate) struct PendingElicitation {
    pub request_id: String,
    pub answered: bool,
    pub request: Result<elicitation::Request, Failure>,
}

impl From<pb::PendingElicitation> for PendingElicitation {
    fn from(pending: pb::PendingElicitation) -> Self {
        let request = decode_request(&pending);
        Self {
            request_id: pending.request_id,
            answered: pending.answered,
            request,
        }
    }
}

fn decode_request(pending: &pb::PendingElicitation) -> Result<elicitation::Request, Failure> {
    let payload = pending
        .request
        .as_ref()
        .ok_or(Failure::Condition(Condition::MissingElicitationRequest))?;
    let raw = serde_json::from_slice(&payload.json)
        .map_err(|error| Failure::InvalidPayload(error.to_string()))?;
    elicitation::parse_request(&raw).map_err(|error| Failure::InvalidPayload(error.to_string()))
}
