use std::collections::{HashMap, HashSet, VecDeque};

/// A module entry for dependency resolution
#[derive(Debug, Clone)]
pub struct ResolverModule {
    /// Module name
    pub name: String,
    /// Module version
    pub version: String,
    /// Dependencies: (name, version_constraint)
    pub dependencies: Vec<(String, String)>,
}

/// Result of dependency resolution
#[derive(Debug, Clone)]
pub struct ResolutionResult {
    /// Modules in install order (topologically sorted)
    pub install_order: Vec<ResolverModule>,
}

/// Errors specific to dependency resolution
#[derive(Debug, thiserror::Error)]
pub enum ResolverError {
    #[error("circular dependency detected: {0}")]
    CircularDependency(String),

    #[error("module not found: {0}")]
    ModuleNotFound(String),

    #[error("version conflict for {module}: required {required}, available {available}")]
    VersionConflict {
        module: String,
        required: String,
        available: String,
    },
}

/// Dependency resolver — resolves module dependencies and produces install order
pub struct DependencyResolver {
    /// Known modules by name
    modules: HashMap<String, ResolverModule>,
}

impl DependencyResolver {
    pub fn new() -> Self {
        Self {
            modules: HashMap::new(),
        }
    }

    /// Register an available module
    pub fn add_module(&mut self, module: ResolverModule) {
        self.modules.insert(module.name.clone(), module);
    }

    /// Resolve a list of required module names into an install order
    pub fn resolve(&self, required: &[String]) -> Result<ResolutionResult, ResolverError> {
        // First, check that all required modules exist
        for name in required {
            if !self.modules.contains_key(name) {
                return Err(ResolverError::ModuleNotFound(name.clone()));
            }
        }

        // Build the full dependency graph
        let mut all_deps = HashSet::new();
        let mut queue: VecDeque<String> = required.iter().cloned().collect();

        while let Some(name) = queue.pop_front() {
            if all_deps.contains(&name) {
                continue;
            }
            all_deps.insert(name.clone());

            if let Some(module) = self.modules.get(&name) {
                for (dep_name, dep_version) in &module.dependencies {
                    // Check the dependency exists
                    if let Some(dep_module) = self.modules.get(dep_name) {
                        // Validate version constraint
                        if !version_matches(&dep_module.version, dep_version) {
                            return Err(ResolverError::VersionConflict {
                                module: dep_name.clone(),
                                required: dep_version.clone(),
                                available: dep_module.version.clone(),
                            });
                        }
                        queue.push_back(dep_name.clone());
                    } else {
                        return Err(ResolverError::ModuleNotFound(dep_name.clone()));
                    }
                }
            }
        }

        // Topological sort using Kahn's algorithm
        let mut in_degree: HashMap<String, usize> = HashMap::new();
        let mut adj: HashMap<String, Vec<String>> = HashMap::new();

        for name in &all_deps {
            in_degree.entry(name.clone()).or_insert(0);
            adj.entry(name.clone()).or_default();
        }

        for name in &all_deps {
            if let Some(module) = self.modules.get(name) {
                for (dep_name, _) in &module.dependencies {
                    if all_deps.contains(dep_name) {
                        adj.entry(dep_name.clone())
                            .or_default()
                            .push(name.clone());
                        *in_degree.entry(name.clone()).or_insert(0) += 1;
                    }
                }
            }
        }

        let mut queue: VecDeque<String> = in_degree
            .iter()
            .filter(|(_, &deg)| deg == 0)
            .map(|(name, _)| name.clone())
            .collect();

        // Sort the initial queue for deterministic output
        let mut sorted_queue: Vec<String> = queue.drain(..).collect();
        sorted_queue.sort();
        queue.extend(sorted_queue);

        let mut install_order = Vec::new();

        while let Some(name) = queue.pop_front() {
            if let Some(module) = self.modules.get(&name) {
                install_order.push(module.clone());
            }

            if let Some(neighbors) = adj.get(&name) {
                let mut sorted_neighbors: Vec<&String> = neighbors.iter().collect();
                sorted_neighbors.sort();
                for neighbor in sorted_neighbors {
                    if let Some(deg) = in_degree.get_mut(neighbor) {
                        *deg -= 1;
                        if *deg == 0 {
                            queue.push_back(neighbor.clone());
                        }
                    }
                }
            }
        }

        // Check for circular dependencies
        if install_order.len() != all_deps.len() {
            let remaining: Vec<String> = all_deps
                .iter()
                .filter(|name| !install_order.iter().any(|m| &m.name == *name))
                .cloned()
                .collect();
            return Err(ResolverError::CircularDependency(remaining.join(", ")));
        }

        Ok(ResolutionResult { install_order })
    }
}

impl Default for DependencyResolver {
    fn default() -> Self {
        Self::new()
    }
}

/// Check if an available version matches a version constraint
///
/// Supports:
/// - Exact match: "1.2.3"
/// - Caret: "^1.2.3" (compatible with 1.x.x)
/// - Tilde: "~1.2.3" (compatible with 1.2.x)
/// - Wildcard: "*"
/// - Partial: "1.2" (treated as ^1.2.0)
pub fn version_matches(available: &str, constraint: &str) -> bool {
    let constraint = constraint.trim();

    // Wildcard matches everything
    if constraint == "*" {
        return true;
    }

    // Parse the constraint prefix
    if let Some(stripped) = constraint.strip_prefix('^') {
        return caret_match(available, stripped);
    }

    if let Some(stripped) = constraint.strip_prefix('~') {
        return tilde_match(available, stripped);
    }

    // Partial version (e.g., "1.2") treated as caret
    let constraint_parts: Vec<&str> = constraint.split('.').collect();
    if constraint_parts.len() < 3 {
        return caret_match(available, constraint);
    }

    // Exact match
    available == constraint
}

/// Caret match: ^1.2.3 matches >=1.2.3 and <2.0.0
fn caret_match(available: &str, constraint: &str) -> bool {
    let avail = parse_version_parts(available);
    let constr = parse_version_parts(constraint);

    if avail.is_none() || constr.is_none() {
        return false;
    }

    let (a_major, a_minor, a_patch) = avail.unwrap();
    let (c_major, c_minor, c_patch) = constr.unwrap();

    if a_major != c_major {
        return false;
    }

    if a_major == 0 {
        // For 0.x versions, minor must match
        if a_minor != c_minor {
            return false;
        }
        return a_patch >= c_patch;
    }

    // Major matches, check >=
    if a_minor > c_minor {
        return true;
    }
    if a_minor == c_minor {
        return a_patch >= c_patch;
    }
    false
}

/// Tilde match: ~1.2.3 matches >=1.2.3 and <1.3.0
fn tilde_match(available: &str, constraint: &str) -> bool {
    let avail = parse_version_parts(available);
    let constr = parse_version_parts(constraint);

    if avail.is_none() || constr.is_none() {
        return false;
    }

    let (a_major, a_minor, a_patch) = avail.unwrap();
    let (c_major, c_minor, c_patch) = constr.unwrap();

    a_major == c_major && a_minor == c_minor && a_patch >= c_patch
}

/// Parse a version string into (major, minor, patch)
fn parse_version_parts(version: &str) -> Option<(u64, u64, u64)> {
    let parts: Vec<&str> = version.split('.').collect();
    let major = parts.first()?.parse::<u64>().ok()?;
    let minor = parts.get(1).and_then(|p| p.parse::<u64>().ok()).unwrap_or(0);
    let patch = parts.get(2).and_then(|p| p.parse::<u64>().ok()).unwrap_or(0);
    Some((major, minor, patch))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_simple_resolve() {
        let mut resolver = DependencyResolver::new();
        resolver.add_module(ResolverModule {
            name: "a".to_string(),
            version: "1.0.0".to_string(),
            dependencies: vec![],
        });
        resolver.add_module(ResolverModule {
            name: "b".to_string(),
            version: "1.0.0".to_string(),
            dependencies: vec![("a".to_string(), "1.0.0".to_string())],
        });

        let result = resolver
            .resolve(&["b".to_string()])
            .unwrap();
        assert_eq!(result.install_order.len(), 2);
        assert_eq!(result.install_order[0].name, "a");
        assert_eq!(result.install_order[1].name, "b");
    }

    #[test]
    fn test_diamond_dependency() {
        let mut resolver = DependencyResolver::new();
        resolver.add_module(ResolverModule {
            name: "base".to_string(),
            version: "1.0.0".to_string(),
            dependencies: vec![],
        });
        resolver.add_module(ResolverModule {
            name: "left".to_string(),
            version: "1.0.0".to_string(),
            dependencies: vec![("base".to_string(), "^1.0.0".to_string())],
        });
        resolver.add_module(ResolverModule {
            name: "right".to_string(),
            version: "1.0.0".to_string(),
            dependencies: vec![("base".to_string(), "^1.0.0".to_string())],
        });
        resolver.add_module(ResolverModule {
            name: "top".to_string(),
            version: "1.0.0".to_string(),
            dependencies: vec![
                ("left".to_string(), "^1.0.0".to_string()),
                ("right".to_string(), "^1.0.0".to_string()),
            ],
        });

        let result = resolver
            .resolve(&["top".to_string()])
            .unwrap();
        assert_eq!(result.install_order.len(), 4);
        // base must come before left and right, which must come before top
        let pos = |name: &str| {
            result
                .install_order
                .iter()
                .position(|m| m.name == name)
                .unwrap()
        };
        assert!(pos("base") < pos("left"));
        assert!(pos("base") < pos("right"));
        assert!(pos("left") < pos("top"));
        assert!(pos("right") < pos("top"));
    }

    #[test]
    fn test_circular_dependency() {
        let mut resolver = DependencyResolver::new();
        resolver.add_module(ResolverModule {
            name: "a".to_string(),
            version: "1.0.0".to_string(),
            dependencies: vec![("b".to_string(), "*".to_string())],
        });
        resolver.add_module(ResolverModule {
            name: "b".to_string(),
            version: "1.0.0".to_string(),
            dependencies: vec![("a".to_string(), "*".to_string())],
        });

        let result = resolver.resolve(&["a".to_string()]);
        assert!(result.is_err());
        match result.unwrap_err() {
            ResolverError::CircularDependency(_) => {}
            e => panic!("expected CircularDependency, got: {:?}", e),
        }
    }

    #[test]
    fn test_missing_dependency() {
        let mut resolver = DependencyResolver::new();
        resolver.add_module(ResolverModule {
            name: "a".to_string(),
            version: "1.0.0".to_string(),
            dependencies: vec![("missing".to_string(), "*".to_string())],
        });

        let result = resolver.resolve(&["a".to_string()]);
        assert!(result.is_err());
        match result.unwrap_err() {
            ResolverError::ModuleNotFound(name) => assert_eq!(name, "missing"),
            e => panic!("expected ModuleNotFound, got: {:?}", e),
        }
    }

    #[test]
    fn test_missing_required_module() {
        let resolver = DependencyResolver::new();
        let result = resolver.resolve(&["nonexistent".to_string()]);
        assert!(result.is_err());
    }

    #[test]
    fn test_version_exact_match() {
        assert!(version_matches("1.2.3", "1.2.3"));
        assert!(!version_matches("1.2.4", "1.2.3"));
    }

    #[test]
    fn test_version_caret() {
        assert!(version_matches("1.2.3", "^1.2.0"));
        assert!(version_matches("1.3.0", "^1.2.0"));
        assert!(!version_matches("2.0.0", "^1.2.0"));
        assert!(!version_matches("1.1.0", "^1.2.0"));
    }

    #[test]
    fn test_version_tilde() {
        assert!(version_matches("1.2.3", "~1.2.0"));
        assert!(version_matches("1.2.9", "~1.2.0"));
        assert!(!version_matches("1.3.0", "~1.2.0"));
        assert!(!version_matches("2.0.0", "~1.2.0"));
    }

    #[test]
    fn test_version_wildcard() {
        assert!(version_matches("1.2.3", "*"));
        assert!(version_matches("0.0.1", "*"));
    }

    #[test]
    fn test_version_partial() {
        assert!(version_matches("1.2.0", "1.2"));
        assert!(version_matches("1.3.0", "1.2"));
        assert!(!version_matches("2.0.0", "1.2"));
    }

    #[test]
    fn test_version_conflict() {
        let mut resolver = DependencyResolver::new();
        resolver.add_module(ResolverModule {
            name: "dep".to_string(),
            version: "2.0.0".to_string(),
            dependencies: vec![],
        });
        resolver.add_module(ResolverModule {
            name: "consumer".to_string(),
            version: "1.0.0".to_string(),
            dependencies: vec![("dep".to_string(), "^1.0.0".to_string())],
        });

        let result = resolver.resolve(&["consumer".to_string()]);
        assert!(result.is_err());
        match result.unwrap_err() {
            ResolverError::VersionConflict { module, .. } => assert_eq!(module, "dep"),
            e => panic!("expected VersionConflict, got: {:?}", e),
        }
    }

    #[test]
    fn test_no_dependencies() {
        let mut resolver = DependencyResolver::new();
        resolver.add_module(ResolverModule {
            name: "standalone".to_string(),
            version: "1.0.0".to_string(),
            dependencies: vec![],
        });

        let result = resolver
            .resolve(&["standalone".to_string()])
            .unwrap();
        assert_eq!(result.install_order.len(), 1);
        assert_eq!(result.install_order[0].name, "standalone");
    }
}
