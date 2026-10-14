use crate::agent::{AgentSpec, Error};
use agent_client_protocol::{AcpAgent, AcpAgentConfig};

/// # Errors
/// Returns an error when the external agent has no command.
pub(super) fn agent(spec: &AgentSpec) -> Result<AcpAgent, Error> {
    let command = spec
        .command()
        .ok_or_else(|| Error::backend(spec.id.clone(), "missing command line"))?;
    Ok(AcpAgent::new(
        AcpAgentConfig::new(super::resolve_program(spec).ok_or_else(|| {
            Error::backend(spec.id.clone(), "executable is missing or not executable")
        })?)
        .args(command.args.clone())
        .envs(spec.env.clone()),
    ))
}
