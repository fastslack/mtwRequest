use dashmap::DashMap;
use mtw_core::MtwError;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EmailTemplate {
    pub id: String, pub name: String, pub subject: String,
    pub body: String, pub body_html: String, pub category: String,
    pub variables: HashMap<String, String>, pub account_id: Option<String>,
    pub created_at: String, pub updated_at: String,
}

pub struct TemplateEngine { templates: DashMap<String, EmailTemplate> }

impl TemplateEngine {
    pub fn new() -> Self { Self { templates: DashMap::new() } }

    pub fn register(&self, t: EmailTemplate) { self.templates.insert(t.id.clone(), t); }
    pub fn get(&self, id: &str) -> Option<EmailTemplate> { self.templates.get(id).map(|t| t.clone()) }
    pub fn list(&self) -> Vec<EmailTemplate> { self.templates.iter().map(|e| e.value().clone()).collect() }

    pub fn render(&self, id: &str, vars: &HashMap<String, String>) -> Result<(String, String, String), MtwError> {
        let t = self.get(id).ok_or_else(|| MtwError::Internal(format!("template not found: {}", id)))?;
        Ok((substitute(&t.subject, vars), substitute(&t.body, vars), substitute(&t.body_html, vars)))
    }
}

impl Default for TemplateEngine { fn default() -> Self { Self::new() } }

fn substitute(text: &str, vars: &HashMap<String, String>) -> String {
    let mut r = text.to_string();
    for (k, v) in vars { r = r.replace(&format!("{{{{{}}}}}", k), v); }
    r
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn test_render() {
        let e = TemplateEngine::new();
        e.register(EmailTemplate {
            id: "t1".into(), name: "W".into(), subject: "Hi {{name}}".into(),
            body: "Hello {{name}}".into(), body_html: "<b>{{name}}</b>".into(),
            category: "".into(), variables: HashMap::new(), account_id: None,
            created_at: "0".into(), updated_at: "0".into(),
        });
        let mut v = HashMap::new();
        v.insert("name".into(), "Alice".into());
        let (s, b, h) = e.render("t1", &v).unwrap();
        assert_eq!(s, "Hi Alice");
        assert_eq!(b, "Hello Alice");
        assert_eq!(h, "<b>Alice</b>");
    }
}
