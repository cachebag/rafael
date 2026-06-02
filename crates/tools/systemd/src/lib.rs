use std::{collections::BTreeMap, fmt, str::FromStr, time::Duration};

use serde::{Deserialize, Deserializer, Serialize, Serializer, de};
use shell::{CommandOutput, CommandRunner, CommandSpec, ShellError};
use thiserror::Error;

#[derive(Debug, Error)]
pub enum SystemdError {
    #[error("systemctl command failed: {0}")]
    Shell(#[from] ShellError),
    #[error("unit name must be non-empty")]
    EmptyUnit,
    #[error("unsafe unit name `{0}`")]
    UnsafeUnit(String),
}

#[derive(Debug, Clone)]
pub struct SystemdClient {
    runner: CommandRunner,
    program: String,
    scope: SystemdScope,
}

impl SystemdClient {
    pub fn user() -> Self {
        Self::new(SystemdScope::User)
    }

    pub fn system() -> Self {
        Self::new(SystemdScope::System)
    }

    pub fn new(scope: SystemdScope) -> Self {
        Self {
            runner: CommandRunner::default(),
            program: "systemctl".to_owned(),
            scope,
        }
    }

    pub fn with_runner(scope: SystemdScope, runner: CommandRunner) -> Self {
        Self {
            runner,
            program: "systemctl".to_owned(),
            scope,
        }
    }

    pub async fn raw<I, S>(&self, args: I) -> Result<CommandOutput, SystemdError>
    where
        I: IntoIterator<Item = S>,
        S: Into<String>,
    {
        Ok(self.runner.run(self.spec(args)?).await?)
    }

    pub async fn show_unit(&self, unit: &UnitName) -> Result<UnitStatus, SystemdError> {
        let output = self
            .runner
            .run_required(self.spec([
                "show".to_owned(),
                unit.as_str().to_owned(),
                "--property=Id".to_owned(),
                "--property=LoadState".to_owned(),
                "--property=ActiveState".to_owned(),
                "--property=SubState".to_owned(),
                "--property=FragmentPath".to_owned(),
                "--no-pager".to_owned(),
            ])?)
            .await?;
        Ok(UnitStatus::from_properties(parse_properties(&output.stdout)))
    }

    pub async fn is_active(&self, unit: &UnitName) -> Result<UnitActive, SystemdError> {
        let output = self
            .runner
            .run(self.spec(["is-active".to_owned(), unit.as_str().to_owned()])?)
            .await?;
        Ok(UnitActive {
            state: output.stdout.trim().to_owned(),
            success: output.success,
        })
    }

    pub async fn start(&self, unit: &UnitName) -> Result<(), SystemdError> {
        self.runner
            .run_required(self.spec(["start".to_owned(), unit.as_str().to_owned()])?)
            .await?;
        Ok(())
    }

    pub async fn stop(&self, unit: &UnitName) -> Result<(), SystemdError> {
        self.runner
            .run_required(self.spec(["stop".to_owned(), unit.as_str().to_owned()])?)
            .await?;
        Ok(())
    }

    pub async fn restart(&self, unit: &UnitName) -> Result<(), SystemdError> {
        self.runner
            .run_required(self.spec(["restart".to_owned(), unit.as_str().to_owned()])?)
            .await?;
        Ok(())
    }

    pub async fn list_units(
        &self,
        unit_type: Option<&str>,
    ) -> Result<Vec<UnitListItem>, SystemdError> {
        let mut args = vec![
            "list-units".to_owned(),
            "--all".to_owned(),
            "--no-legend".to_owned(),
            "--plain".to_owned(),
            "--no-pager".to_owned(),
        ];
        if let Some(unit_type) = unit_type {
            args.push(format!("--type={unit_type}"));
        }
        let output = self.runner.run_required(self.spec(args)?).await?;
        Ok(parse_unit_list(&output.stdout))
    }

    fn spec<I, S>(&self, args: I) -> Result<CommandSpec, ShellError>
    where
        I: IntoIterator<Item = S>,
        S: Into<String>,
    {
        let mut scoped_args = Vec::new();
        if self.scope == SystemdScope::User {
            scoped_args.push("--user".to_owned());
        }
        scoped_args.extend(args.into_iter().map(Into::into));
        Ok(CommandSpec::new(self.program.clone())?
            .args(scoped_args)
            .timeout(Duration::from_secs(60)))
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SystemdScope {
    User,
    System,
}

#[derive(Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct UnitName(String);

impl UnitName {
    pub fn parse(value: impl Into<String>) -> Result<Self, SystemdError> {
        let value = value.into();
        if value.is_empty() {
            return Err(SystemdError::EmptyUnit);
        }
        if value.starts_with('-')
            || value.contains('\0')
            || value.contains('/')
            || value.chars().any(char::is_whitespace)
        {
            return Err(SystemdError::UnsafeUnit(value));
        }
        Ok(Self(value))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Debug for UnitName {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_tuple("UnitName").field(&self.0).finish()
    }
}

impl fmt::Display for UnitName {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

impl FromStr for UnitName {
    type Err = SystemdError;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        Self::parse(value)
    }
}

impl Serialize for UnitName {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        serializer.serialize_str(&self.0)
    }
}

impl<'de> Deserialize<'de> for UnitName {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let value = String::deserialize(deserializer)?;
        Self::parse(value).map_err(de::Error::custom)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct UnitStatus {
    pub id: Option<String>,
    pub load_state: Option<String>,
    pub active_state: Option<String>,
    pub sub_state: Option<String>,
    pub fragment_path: Option<String>,
    #[serde(default)]
    pub properties: BTreeMap<String, String>,
}

impl UnitStatus {
    fn from_properties(properties: BTreeMap<String, String>) -> Self {
        Self {
            id: properties.get("Id").cloned(),
            load_state: properties.get("LoadState").cloned(),
            active_state: properties.get("ActiveState").cloned(),
            sub_state: properties.get("SubState").cloned(),
            fragment_path: properties.get("FragmentPath").cloned(),
            properties,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct UnitActive {
    pub state: String,
    pub success: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct UnitListItem {
    pub unit: String,
    pub load: String,
    pub active: String,
    pub sub: String,
    pub description: String,
}

fn parse_properties(stdout: &str) -> BTreeMap<String, String> {
    stdout
        .lines()
        .filter_map(|line| {
            let (key, value) = line.split_once('=')?;
            Some((key.to_owned(), value.to_owned()))
        })
        .collect()
}

fn parse_unit_list(stdout: &str) -> Vec<UnitListItem> {
    stdout
        .lines()
        .filter_map(|line| {
            let mut parts = line.split_whitespace();
            let unit = parts.next()?.to_owned();
            let load = parts.next()?.to_owned();
            let active = parts.next()?.to_owned();
            let sub = parts.next()?.to_owned();
            let description = parts.collect::<Vec<_>>().join(" ");
            Some(UnitListItem {
                unit,
                load,
                active,
                sub,
                description,
            })
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn validates_unit_names() {
        assert!(UnitName::parse("llama-server.service").is_ok());
        assert!(UnitName::parse("-bad.service").is_err());
        assert!(UnitName::parse("bad service").is_err());
    }

    #[test]
    fn parses_show_properties() {
        let status = UnitStatus::from_properties(parse_properties(
            "Id=llama.service\nLoadState=loaded\nActiveState=active\n",
        ));

        assert_eq!(status.id.as_deref(), Some("llama.service"));
        assert_eq!(status.active_state.as_deref(), Some("active"));
    }

    #[test]
    fn parses_list_units() {
        let units = parse_unit_list("a.service loaded active running test service\n");

        assert_eq!(units[0].unit, "a.service");
        assert_eq!(units[0].description, "test service");
    }
}
