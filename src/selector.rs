use anyhow::{Result, anyhow, bail};

use crate::model::LiveNode;
#[cfg(test)]
use crate::model::UiNode;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Selector {
    pub role: Option<String>,
    pub name: Option<NameMatch>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum NameMatch {
    Exact(String),
    Contains(String),
}

impl Selector {
    pub fn parse(input: &str) -> Result<Self> {
        let raw = input.trim();
        if raw.is_empty() {
            bail!("selector must not be empty");
        }

        if let Some(name) = raw.strip_prefix('~') {
            let name = normalize_name(name)?;
            return Ok(Self {
                role: None,
                name: Some(NameMatch::Contains(name)),
            });
        }

        if let Some((role, name)) = raw.split_once('~') {
            let role = normalize_role(role)?;
            let name = normalize_name(name)?;
            return Ok(Self {
                role: Some(role),
                name: Some(NameMatch::Contains(name)),
            });
        }

        if let Some((role, name)) = raw.split_once(':') {
            let role = normalize_role(role)?;
            let name = normalize_name(name)?;
            return Ok(Self {
                role: Some(role),
                name: Some(NameMatch::Exact(name)),
            });
        }

        Ok(Self {
            role: Some(normalize_role(raw)?),
            name: None,
        })
    }

    pub fn matches_live(&self, node: &LiveNode) -> bool {
        self.matches_parts(&node.role, node.name.as_deref())
    }

    #[cfg(test)]
    pub fn matches(&self, node: &UiNode) -> bool {
        self.matches_parts(&node.role, node.name.as_deref())
    }

    fn matches_parts(&self, role: &str, name: Option<&str>) -> bool {
        if let Some(expected_role) = &self.role {
            let actual_role = normalize_role_for_match(role);
            if !roles_are_compatible(expected_role, &actual_role) {
                return false;
            }
        }

        if let Some(expected_name) = &self.name {
            let actual_name = name.map(normalize_for_match).unwrap_or_default();

            match expected_name {
                NameMatch::Exact(expected) => actual_name == *expected,
                NameMatch::Contains(expected) => contains_name_match(&actual_name, expected),
            }
        } else {
            true
        }
    }
}

pub fn parse_selector_chain(raw_selectors: &[String]) -> Result<Vec<Selector>> {
    raw_selectors
        .iter()
        .map(|value| Selector::parse(value))
        .collect()
}

fn normalize_role(input: &str) -> Result<String> {
    let normalized = canonical_role_name(&normalize_for_match(input));
    if normalized.is_empty() {
        return Err(anyhow!("selector role must not be empty"));
    }
    Ok(normalized)
}

fn normalize_role_for_match(input: &str) -> String {
    canonical_role_name(&normalize_for_match(input))
}

fn normalize_name(input: &str) -> Result<String> {
    let normalized = normalize_for_match(input);
    if normalized.is_empty() {
        return Err(anyhow!("selector name must not be empty"));
    }
    Ok(normalized)
}

fn normalize_for_match(input: &str) -> String {
    input
        .trim()
        .to_ascii_lowercase()
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
}

fn canonical_role_name(role: &str) -> String {
    match role {
        "button" => "push button".to_string(),
        "pushbutton" => "push button".to_string(),
        "textbox" | "text box" | "text field" | "input" => "entry".to_string(),
        "document" => "document web".to_string(),
        "checkbox" | "check box" => "check box".to_string(),
        "radio" | "radio button" => "radio button".to_string(),
        "combobox" | "combo box" | "select" => "combo box".to_string(),
        "iframe" | "inline frame" | "internal frame" | "frame" => "frame".to_string(),
        other => other.to_string(),
    }
}

fn roles_are_compatible(expected: &str, actual: &str) -> bool {
    // Roles match exactly. Treating `entry` and `combo box` as interchangeable
    // silently widened selectors: `Combo Box --nth 2` could land on a date
    // `entry`, so positional matching changed meaning as a form changed state.
    expected == actual
}

fn contains_name_match(actual: &str, expected: &str) -> bool {
    if actual.contains(expected) {
        return true;
    }

    let actual_tokens = actual.split_whitespace().collect::<Vec<_>>();
    let expected_tokens = expected.split_whitespace().collect::<Vec<_>>();

    !expected_tokens.is_empty()
        && expected_tokens
            .iter()
            .all(|token| actual_tokens.iter().any(|actual| actual.contains(token)))
}

#[cfg(test)]
mod tests {
    use super::{NameMatch, Selector};
    use crate::model::UiNode;

    #[test]
    fn parses_exact_role_and_name() {
        let selector = Selector::parse("Push Button:Back").unwrap();
        assert_eq!(selector.role.as_deref(), Some("push button"));
        assert_eq!(selector.name, Some(NameMatch::Exact("back".into())));
    }

    #[test]
    fn parses_role_only() {
        let selector = Selector::parse("Entry").unwrap();
        assert_eq!(selector.role.as_deref(), Some("entry"));
        assert_eq!(selector.name, None);
    }

    #[test]
    fn parses_contains_name_without_role() {
        let selector = Selector::parse("~search").unwrap();
        assert_eq!(selector.role, None);
        assert_eq!(selector.name, Some(NameMatch::Contains("search".into())));
    }

    #[test]
    fn parses_role_and_contains_name() {
        let selector = Selector::parse("Toggle Button~search").unwrap();
        assert_eq!(selector.role.as_deref(), Some("toggle button"));
        assert_eq!(selector.name, Some(NameMatch::Contains("search".into())));
    }

    #[test]
    fn exact_match_requires_same_role_and_name() {
        let selector = Selector::parse("Push Button:OK").unwrap();
        let ok = UiNode::new("Push Button", Some("OK".into()), vec![]);
        let cancel = UiNode::new("Push Button", Some("Cancel".into()), vec![]);
        let label = UiNode::new("Label", Some("OK".into()), vec![]);

        assert!(selector.matches(&ok));
        assert!(!selector.matches(&cancel));
        assert!(!selector.matches(&label));
    }

    #[test]
    fn contains_match_is_case_and_whitespace_insensitive() {
        let selector = Selector::parse("Entry~Address   Bar").unwrap();
        let node = UiNode::new("Entry", Some("Address and Search Bar".into()), vec![]);
        assert!(selector.matches(&node));
    }

    #[test]
    fn parses_role_aliases() {
        let button = Selector::parse("Button:Submit").unwrap();
        assert_eq!(button.role.as_deref(), Some("push button"));

        let textbox = Selector::parse("Text Box:Name").unwrap();
        assert_eq!(textbox.role.as_deref(), Some("entry"));

        let document = Selector::parse("Document:Example").unwrap();
        assert_eq!(document.role.as_deref(), Some("document web"));

        let frame = Selector::parse("Iframe:Checkout").unwrap();
        assert_eq!(frame.role.as_deref(), Some("frame"));
    }

    #[test]
    fn matches_internal_frame_role_via_frame_alias() {
        let selector = Selector::parse("Frame:Demo Frame").unwrap();
        let node = UiNode::new("Internal Frame", Some("Demo Frame".into()), vec![]);
        assert!(selector.matches(&node));
    }

    #[test]
    fn roles_match_exactly() {
        // `entry` and `combo box` are distinct roles, so a `Combo Box` selector
        // must not also match date or text entries. Widening this silently
        // changed which field `--nth` selected.
        let combo = Selector::parse("Combo Box:Rech.").unwrap();
        assert!(combo.matches(&UiNode::new("Combo Box", Some("Rech.".into()), vec![])));
        assert!(!combo.matches(&UiNode::new("Entry", Some("Rech.".into()), vec![])));

        let entry = Selector::parse("Text Box:Rech.").unwrap();
        assert!(entry.matches(&UiNode::new("Entry", Some("Rech.".into()), vec![])));
        assert!(!entry.matches(&UiNode::new("Combo Box", Some("Rech.".into()), vec![])));
    }
}
