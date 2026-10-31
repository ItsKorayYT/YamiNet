//! Structured PASS / FAIL / NOT RUN reports shared by every testnet command.
//!
//! Reports intentionally contain public identifiers and operator-chosen paths
//! only. They never contain key bytes, document text or raw remote errors.
use serde::Serialize;
use serde_json::{json, Value};

pub const SCHEMA: &str = "yaminet-testnet-report-1";
pub const PREFLIGHT_SCHEMA: &str = "yaminet-testnet-preflight-1";
const MAX_DETAIL: usize = 640;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "UPPERCASE")]
pub enum Status {
    Pass,
    Fail,
    NotRun,
}

impl Status {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Pass => "PASS",
            Self::Fail => "FAIL",
            Self::NotRun => "NOT RUN",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Outcome {
    Pass,
    Fail,
    Incomplete,
}

impl Outcome {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Pass => "PASS",
            Self::Fail => "FAIL",
            Self::Incomplete => "NOT RUN",
        }
    }

    pub fn exit_code(self) -> i32 {
        match self {
            Self::Pass => 0,
            Self::Fail => 1,
            Self::Incomplete => 2,
        }
    }
}

/// One operator-visible check. `required` controls whether a NOT RUN result
/// makes the whole report incomplete; it is not serialized.
#[derive(Debug, Clone)]
pub struct Check {
    pub id: &'static str,
    pub title: &'static str,
    pub status: Status,
    pub detail: String,
    pub required: bool,
    pub data: Option<Value>,
}

impl Check {
    pub fn new(
        id: &'static str,
        title: &'static str,
        status: Status,
        detail: impl Into<String>,
    ) -> Self {
        Self {
            id,
            title,
            status,
            detail: sanitize(&detail.into()),
            required: true,
            data: None,
        }
    }

    pub fn pass(id: &'static str, title: &'static str, detail: impl Into<String>) -> Self {
        Self::new(id, title, Status::Pass, detail)
    }

    pub fn fail(id: &'static str, title: &'static str, detail: impl Into<String>) -> Self {
        Self::new(id, title, Status::Fail, detail)
    }

    pub fn not_run(id: &'static str, title: &'static str, detail: impl Into<String>) -> Self {
        Self::new(id, title, Status::NotRun, detail)
    }

    pub fn optional(mut self) -> Self {
        self.required = false;
        self
    }

    pub fn with_data(mut self, data: Value) -> Self {
        self.data = Some(data);
        self
    }
}

/// Replace control characters, collapse whitespace and bound free text.
fn sanitize(text: &str) -> String {
    let mut output = String::with_capacity(text.len().min(MAX_DETAIL));
    let mut space = false;
    for character in text.chars() {
        if character.is_control() || character.is_whitespace() {
            space = true;
            continue;
        }
        if space && !output.is_empty() {
            output.push(' ');
        }
        space = false;
        if output.chars().count() >= MAX_DETAIL {
            output.push_str("...");
            break;
        }
        output.push(character);
    }
    output
}

#[derive(Debug, Clone)]
pub struct Report {
    schema: &'static str,
    title: &'static str,
    target: Value,
    checks: Vec<Check>,
}

impl Report {
    pub fn new(title: &'static str, target: Value) -> Self {
        Self {
            schema: SCHEMA,
            title,
            target,
            checks: Vec::new(),
        }
    }

    pub fn preflight(target: Value) -> Self {
        Self {
            schema: PREFLIGHT_SCHEMA,
            title: "YamiNet testnet preflight",
            target,
            checks: Vec::new(),
        }
    }

    pub fn push(&mut self, check: Check) -> &mut Self {
        self.checks.push(check);
        self
    }

    pub fn checks(&self) -> &[Check] {
        &self.checks
    }

    pub fn has_failure(&self) -> bool {
        self.checks.iter().any(|check| check.status == Status::Fail)
    }

    pub fn outcome(&self) -> Outcome {
        if self.has_failure() {
            Outcome::Fail
        } else if self
            .checks
            .iter()
            .any(|check| check.required && check.status == Status::NotRun)
        {
            Outcome::Incomplete
        } else {
            Outcome::Pass
        }
    }

    pub fn counts(&self) -> (usize, usize, usize) {
        let mut counts = (0, 0, 0);
        for check in &self.checks {
            match check.status {
                Status::Pass => counts.0 += 1,
                Status::Fail => counts.1 += 1,
                Status::NotRun => counts.2 += 1,
            }
        }
        counts
    }

    pub fn to_text(&self) -> String {
        let (passed, failed, not_run) = self.counts();
        let mut text = format!(
            "{} ({} {})\nTarget: {}\nScope: automated local checks only; independent machines and physical topology NOT VERIFIED.\n\n",
            self.title,
            self.schema,
            env!("CARGO_PKG_VERSION"),
            display_target(&self.target)
        );
        for check in &self.checks {
            text.push_str(&format!(
                "[{}] {} - {}\n",
                check.status.as_str(),
                check.title,
                check.detail
            ));
        }
        text.push_str(&format!(
            "\nRESULT: {} - {passed} passed, {failed} failed, {not_run} not run.\n",
            self.outcome().as_str()
        ));
        text.push_str(
            "This report validates software behaviour on this machine. It does not prove real-world\nreachability, provider routing, firewall state, anonymity or production security.\n",
        );
        text
    }

    pub fn to_json(&self) -> Value {
        let checks: Vec<Value> = self
            .checks
            .iter()
            .map(|check| {
                let mut value = json!({
                    "id": check.id,
                    "title": check.title,
                    "status": check.status.as_str(),
                    "detail": check.detail,
                    "required": check.required,
                });
                if let Some(data) = &check.data {
                    value["data"] = data.clone();
                }
                value
            })
            .collect();
        json!({
            "schema": self.schema,
            "version": env!("CARGO_PKG_VERSION"),
            "title": self.title,
            "scope": {
                "validation": "automated local checks only",
                "independent_machines": "NOT VERIFIED",
                "physical_topology": "NOT VERIFIED",
                "note": "Loopback or single-machine results cannot prove VPS, home-router or independent-client deployment."
            },
            "target": self.target,
            "result": self.outcome().as_str(),
            "complete": self.outcome() != Outcome::Incomplete,
            "counts": {
                "pass": self.counts().0,
                "fail": self.counts().1,
                "not_run": self.counts().2,
            },
            "checks": checks,
        })
    }
}

fn display_target(target: &Value) -> String {
    let destination = target
        .get("destination")
        .and_then(Value::as_str)
        .unwrap_or("(not selected)");
    let document = target
        .get("document")
        .and_then(Value::as_str)
        .unwrap_or("(none)");
    let asset = target.get("asset").and_then(Value::as_str);
    match asset {
        Some(asset) => format!("{destination}/{document} (asset {asset})"),
        None => format!("{destination}/{document}"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn outcome_requires_every_mandatory_check_and_never_hides_failures() {
        let mut report = Report::new("test", json!({"destination":"yn1-example"}));
        report.push(Check::pass("a", "A", "ok"));
        report.push(Check::not_run("b", "B", "not run").optional());
        assert_eq!(report.outcome(), Outcome::Pass);
        report.push(Check::not_run("c", "C", "required missing"));
        assert_eq!(report.outcome(), Outcome::Incomplete);
        assert_eq!(report.outcome().exit_code(), 2);
        report.push(Check::fail("d", "D", "broken"));
        assert_eq!(report.outcome(), Outcome::Fail);
        assert_eq!(report.outcome().exit_code(), 1);
    }

    #[test]
    fn reports_are_bounded_redacted_and_structured() {
        let mut report = Report::new(
            "test",
            json!({"destination":"yn1-example","document":"home"}),
        );
        report.push(Check::fail(
            "x",
            "Injected",
            format!("private-key-text\n{}", "x".repeat(2000)),
        ));
        let text = report.to_text();
        assert!(!text.contains("private-key-text\n"));
        assert!(text.len() < 2500);
        let json = report.to_json();
        assert_eq!(json["schema"], SCHEMA);
        assert_eq!(json["result"], "FAIL");
        assert_eq!(json["checks"][0]["status"], "FAIL");
        assert_eq!(json["scope"]["independent_machines"], "NOT VERIFIED");
        assert!(!json.to_string().contains('\n'));
    }
}
