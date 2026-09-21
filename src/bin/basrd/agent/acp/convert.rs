use crate::agent::{AgentSpec, Error};
use agent_client_protocol::{AcpAgent, AcpAgentConfig};

impl TryFrom<&AgentSpec> for AcpAgent {
    type Error = Error;

    /// # Errors
    /// Returns an error when the external agent has no command.
    fn try_from(spec: &AgentSpec) -> Result<Self, Self::Error> {
        let command = spec
            .command()
            .ok_or_else(|| Error::backend(spec.id.clone(), "missing command line"))?;
        Ok(Self::new(
            AcpAgentConfig::new(command.program.clone())
                .args(command.args.clone())
                .envs(spec.env.clone()),
        ))
    }
}
