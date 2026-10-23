//! Trusted application boundaries for interactive authentication invocations.

use std::{
    collections::BTreeMap,
    path::{Path, PathBuf},
};

use anyhow::Context;
use bas::{
    authentication::{Arguments, Execution, Invocation, Method},
    config::{AgentSpec, ManagedAgent},
};
use serde::{Deserialize, Serialize};

use super::{Record, installation, registry};

#[derive(Clone, Debug, PartialEq, Eq, Deserialize, Serialize)]
pub(super) struct LaunchBoundary {
    program: PathBuf,
    application: PathBuf,
    bootstrap_args: Vec<String>,
    application_args: Vec<String>,
}

impl LaunchBoundary {
    pub(super) fn from_record(record: &Record) -> Option<Self> {
        let command = record.spec.command()?;
        let (application, bootstrap_args, application_args) = match record.manifest.select().ok()? {
            registry::Selected::Binary(binary) => {
                let application = record
                    .directory
                    .join(registry::safe_relative(&binary.cmd).ok()?);
                if command.program != application || command.args != binary.args {
                    return None;
                }
                (application, Vec::new(), binary.args.clone())
            }
            registry::Selected::Npm(npm) => {
                if let Some(runtime) = &record.runtime
                    && command.program == runtime.node
                    && command.args.len() == npm.args.len() + 1
                    && command.args[1..] == npm.args
                {
                    let application = PathBuf::from(&command.args[0]);
                    if !application.starts_with(&record.directory) || !application.is_file() {
                        return None;
                    }
                    (application, command.args[..1].to_vec(), npm.args.clone())
                } else if command.program.starts_with(&record.directory) && command.args == npm.args
                {
                    (command.program.clone(), Vec::new(), npm.args.clone())
                } else {
                    return None;
                }
            }
        };
        Some(Self {
            program: command.program.clone(),
            application,
            bootstrap_args,
            application_args,
        })
    }

    fn matches(&self, spec: &AgentSpec) -> bool {
        spec.command().is_some_and(|command| {
            command.program == self.program
                && command
                    .args
                    .iter()
                    .eq(self.bootstrap_args.iter().chain(&self.application_args))
        })
    }

    /// # Errors
    /// Rejects edited or inconsistent installation boundaries.
    fn checked(record: &Record, spec: &AgentSpec) -> Result<Self, Error> {
        let boundary = Self::from_record(record)
            .filter(|boundary| boundary.matches(spec))
            .ok_or(Error::UnknownBoundary)?;
        if record
            .launch
            .as_ref()
            .is_some_and(|stored| stored != &boundary)
        {
            return Err(Error::UnknownBoundary);
        }
        Ok(boundary)
    }
}

#[derive(Debug, thiserror::Error)]
enum Error {
    #[error("Selected method does not describe terminal authentication")]
    NotTerminal,
    #[error("Authentication command differs from the installed agent application")]
    CommandMismatch,
    #[error(
        "The application's launcher boundary is unknown; open the agent's configuration manually"
    )]
    UnknownBoundary,
    #[error("Agent command contains invalid process arguments")]
    InvalidCommand,
}

/// Builds a literal terminal invocation from a freshly checked descriptor.
///
/// # Errors
/// Rejects unsupported methods, mismatched commands, invalid process arguments,
/// or legacy application arguments without a trusted launcher boundary.
pub fn invocation(
    spec: &AgentSpec,
    managed: Option<&ManagedAgent>,
    program: &Path,
    method: &Method,
) -> anyhow::Result<Invocation> {
    // Provenance comes from the active catalog, never a matching executable or
    // a global "last installed" record. A custom copy cannot inherit this trust.
    let record = managed.map(installation::load_record).transpose()?;
    if record.as_ref().is_some_and(|record| record.spec != *spec) {
        return Err(Error::CommandMismatch.into());
    }
    build(spec, program, method, record.as_ref())
}

/// # Errors
/// Rejects a terminal descriptor that cannot reproduce this agent's invocation.
fn build(
    spec: &AgentSpec,
    program: &Path,
    method: &Method,
    record: Option<&Record>,
) -> anyhow::Result<Invocation> {
    let Execution::Terminal(terminal) = &method.execution else {
        return Err(Error::NotTerminal.into());
    };
    let command = spec.command().context("ACP agent command is missing")?;
    let mut args = match terminal.arguments {
        Arguments::Append => command.args.clone(),
        Arguments::ReplaceApplication => {
            if let Some(record) = record {
                LaunchBoundary::checked(record, spec)?.bootstrap_args
            } else {
                return Err(Error::UnknownBoundary.into());
            }
        }
        Arguments::ReplaceInvocation => {
            let declared = terminal.command.as_deref().ok_or(Error::CommandMismatch)?;
            if let Some(record) = record {
                let boundary = LaunchBoundary::checked(record, spec)?;
                if same_application(declared, program, &spec.env) {
                    if !terminal.args.starts_with(&boundary.bootstrap_args) {
                        return Err(Error::CommandMismatch.into());
                    }
                    Vec::new()
                } else if same_application(declared, &boundary.application, &spec.env) {
                    boundary.bootstrap_args
                } else {
                    return Err(Error::CommandMismatch.into());
                }
            } else if command.args.is_empty() && same_application(declared, program, &spec.env) {
                Vec::new()
            } else {
                return Err(Error::UnknownBoundary.into());
            }
        }
    };
    args.extend(terminal.args.clone());
    let mut env = spec.env.clone();
    env.extend(terminal.env.clone());
    if !program.is_absolute()
        || args.iter().any(|arg| arg.contains('\0'))
        || env
            .iter()
            .any(|(key, value)| key.is_empty() || key.contains(['=', '\0']) || value.contains('\0'))
    {
        return Err(Error::InvalidCommand.into());
    }
    Ok(Invocation {
        program: program.to_owned(),
        args,
        env,
    })
}

fn same_application(declared: &str, application: &Path, env: &BTreeMap<String, String>) -> bool {
    let resolved = which::which_in(
        declared,
        env.get("PATH")
            .map(std::ffi::OsString::from)
            .or_else(|| std::env::var_os("PATH")),
        std::env::current_dir().unwrap_or_default(),
    );
    resolved.ok().is_some_and(|path| {
        path == application
            || std::fs::canonicalize(path)
                .ok()
                .zip(std::fs::canonicalize(application).ok())
                .is_some_and(|(left, right)| left == right)
    })
}

#[cfg(test)]
mod tests;
